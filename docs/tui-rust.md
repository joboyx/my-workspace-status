# Rust TUI

`workspace-status` and `ws` from `crates/workspace-status` open a ratatui TUI when stdout is a TTY (or you pass `-i` / `--tui`) and you did not pass `--plain`, `--json`, `-v`, `-p`, or `-d`. Those headless flags still win over `--tui`.

A non-TTY run without those flags still prints `--plain`. Agents must pass `--plain` or `--json`.

The TypeScript Ink app stays in this repository. Use it when you need a feature that this TUI does not implement yet.

Screenshots of the daily views live in the [root README](../README.md#screenshots).
To rebuild those frames, run `./scripts/capture-demo-stills.sh` (see [demo.md](./demo.md)).

## Daily keys

| Key | Action |
| --- | --- |
| `q` | Quit |
| `?` | Help overlay (short list, not a wall of text) |
| `j` / `k` or arrows | Move the tree. On a focused graph, move the graph cursor. On a file list, move the file. On a focused file diff, scroll the diff |
| `z` | Toggle fold on this row |
| `zz` | First `z` toggles this row immediately and arms a 400ms pending. Second `z` in the window is `toggleSubtree` (no extra toggle). A late second `z` is a new single toggle |
| `t` | Toggle directory tree / flat paths. On the workspace tree this is `tree_mode`. In a commit-files or commit-diff drill this is an independent commit-file tree mode (default tree). Status is `Directory tree` / `Flat paths` |
| `h` / `l` or left / right | Tree focused: close / open fold. Focused file diff: pan left / right. Graph or commit-file list focused: no-op (does not fold the tree). Space does not fold |
| `.` | Show or hide ignored repos |
| `/` | Search the focused pane (workspace tree, graph, commit-file list, or file diff). Enter arms the query. Esc clears. When `?` help is open, `/` opens a help query: characters (including `n`/`N`) append; highlight only; rows stay visible. Esc clears the query (help stays). No Enter-arm and no `n`/`N` next/prev while help search is open. `?` / `q` / Esc with no query still close help |
| `n` / `N` | Next / previous match on the pane bound at `/` (previous is `N`, not `p`). While help search is open these keys append to the query instead |
| `g` / `G` | `gg` (second `g` within ~400ms) moves to the start of the focused list. Lone `g` expires with no move. `G` is the end. Home / End stay as Rust extras |
| `Ctrl+u` / `Ctrl+d` | Move the focused list ±5 rows (tree, graph, or commit-files). On a focused file diff, scroll ±5. PageUp / PageDown stay one viewport |
| `Ctrl+O` | Toggle unlimited `-U` context when the right pane is already a file diff. Fires from a left-focused workspace file or commit-file as well as from the right pane. A second press restores the previous context. No-op on tree, graph, or a commit-file list |
| `s` | Stage dirty files in the focused scope (file, dir, checkout, or flat repo). Workspace and family-container rows are a no-op |
| `u` | Unstage dirty files in the focused scope. Workspace and family-container rows are a no-op |
| `x` | Revert dirty files in the focused scope. Workspace and family-container rows are a no-op. `y` discards tracked (keeps untracked except a sole untracked file). `Y` also deletes untracked |
| `e` | Edit the focused workspace file, commit-file row, or commit-file diff (`$EDITOR` or config `editor`) |
| `f` | Fetch visible targets. Workspace fans out to primaries. No-updates group is a no-op |
| `p` | Pull visible targets that are behind. Workspace / repo / checkout only — silent no-op on a file or dir row. No-updates group is a no-op |
| `d` | Switch visible targets that are off the default branch. Already-default is a no-op (does not pull). Same kinds as `p` — silent no-op on a file or dir row. No-updates group is a no-op |
| `P` | Push the focused visible repo or checkout when it is ahead, diverged, or has no upstream. In-sync is a no-op |
| `S` | Stash menu. Left-pane only (right-focused graph is a silent no-op). Tree dirty file or repo is create-only (`s`). Graph commit or uncommitted (left-focused, e.g. commit drill) offers apply / pop of the latest stash; drop needs a focused stash row (`a` / `p` / `D`) |
| `Enter` | Focus the right pane, or drill: graph commit → commit detail (graph stays left) → commit diff |
| `Esc` | Pop commit diff to commit files (graph still left), then leave the drill so the graph is the right pane again, then focus the tree. Esc never quits |
| `a` / `p` / `D` | Apply / pop / drop the focused graph stash row. Drop asks `y` / `n` |
| `b` | Tree: local branch picker (list, filter, checkout, `C` create at HEAD). Graph commit: checkout refs on that commit (one name checks out, several open a name picker). Dirty tree refuses (`Dirty worktree — commit or stash first`). Origin out-of-sync confirm only for a selected `origin/…` name |
| `c` | Graph commit: create a branch at that commit (`git branch -- name commitId`). No checkout. No-op when a graph commit is not focused |
| `r` | Reload the focused checkout. Workspace row or No-updates group reloads the whole workspace |
| `space` | Mark a dirty workspace-tree file as reviewed (depth 0 only). Does not flip reviewed after a commit drill. Writes the same viewed-files store as the Ink app |
| `w` / `W` | Remove the focused linked worktree after a boxed confirm (`y` / `n`). Other rows refuse with `Focus a linked worktree to remove` |
| `Tab` | Focus the other pane |
| click | Select a tree, graph, or commit-file row, or focus the right pane. Click the fold chevron to toggle that row's fold |
| double-click | Same as Enter / drill on the clicked cell. A chevron double-click still folds |
| `m` | Toggle mouse capture. Off ignores click / drag / wheel |
| drag | Resize the tree / right pane split, or the in-diff side-by-side RULE |
| `i` | Toggle inline / split on a file diff. Split falls back to inline below 100 columns |
| `;` or Ctrl+Space | EasyMotion on the focused list (tree, graph, or commit files). Labels `a`–`z` then `aa`… on the current viewport only. Type a label to jump. Esc cancels. Diff-focused start is a no-op |
| `T` | Cycle the built-in colour theme. Wraps. Seed from `WS_STATUS_THEME`; the cycle stays in this session (same as Ink — there is no theme file) |

`-a` starts with ignored repos shown. `-f` starts a fetch after the first paint. First paint does not wait on a network fetch.

`WS_STATUS_WATCH_MS` polls local git and refreshes the snapshot. Default is `3000`. `0` disables the poll. Fold, focus, and scroll stay put. File rows flash (~800ms) when the git status letter **or** worktree `size:mtimeMs` changes, so an in-place save of an already-modified file flashes. Chrome rows flash when label / fold / repo path moves.

`WS_STATUS_FETCH_MS` runs `git fetch` on every snapshot checkout except hidden ignored repos, including linked worktrees. Default is `300000` (5 minutes). `0` disables it. The watch poll stays a separate timer. Shown ignored repos (`.` / `-a`) are included. Manual `f` stays focus-scoped: primaries on the workspace / family row, and a linked worktree only when that row is focused.

## What this TUI does

- Tree of repos, linked worktrees, and dirty files from the same snapshot builder as `--plain` / `--json`. Chrome matches Ink: status letters on the right, Nerd file/folder/sync glyphs, file-oriented workspace header, branch-labeled linked checkouts
- Files sit in a directory trie by default. `t` toggles tree / flat on the workspace. Status is `Directory tree` / `Flat paths`
- Dir rows fold with `z` / `h` / `l`. First `z` toggles immediately and arms a 400ms `zz` window; second `z` is `toggleSubtree`. A dir `s` / `u` / `x` writes files under that dir
- Tree / flat stays in this session. Rust has no session store. Commit-file lists have their own `t` toggle and the same dir-trie collapse
- Right pane at depth 0: file diff when a dirty file is focused. Graph pane via `workspace-status-graph` when a repo or worktree is focused
- File diffs paint `{repo}/{path}  inline|split` (plus ` · full` / ` · pan N`) then STAGED / UNSTAGED / NEW labels and a line-number gutter. After a commit drill the graph stays left, so the header is the path. Intra-line / syntax highlight stays Ink-only
- Hidden ignored repos stay out of the tree, search, stage / unstage / revert, and fetch / pull / push / default unless you show them
- `/` searches the focused pane. Tree matches include folded rows and unfold ancestors. Graph search matches commit subjects and ref names. Commit-file search matches paths. Diff search matches painted line text. Matching tree, graph, and commit-file rows paint the filter/search background (Ink `searchMatchIds`); the cursor still wins. When `?` help is open, `/` opens a help-local query (highlight only; rows stay visible; `n`/`N` append)
- `Ctrl+O` toggles unlimited unified context when the right pane is already a file diff, including from a left-focused workspace file or commit-file. A second press restores the previous context. No-op on tree, graph, or a commit-file list
- Fetch / pull / default do not fan out to linked worktrees unless the focused row is that worktree. `p` / `d` are silent no-ops on file and dir rows; `f` still fetches the scoped checkout. The No-updates group is a no-op for `f` / `p` / `d`. Workspace `f` / `p` / `d` still targets visible primaries. The background fetch timer still includes linked worktrees (and shown ignored); it skips hidden ignored. While any of those (or `P`) runs, the breadcrumb trailing slot shows `Verb n/N…` and repaints after each repo.
- Tree writes (`s` / `u` / `x` / `f` / `p` / `P` / `d` / `b` / `W`) no-op at commit-files depth ≥ 1 (they must not write the hidden workspace-tree cursor). The same keys, plus `S`, no-op when the right pane is focused unless the allow-list matches: graph `b` / `c` / `a` / `p` / `D` (`GraphCheckout` / create / stash apply-pop-drop), commit-file nav, and diff `e` / `Ctrl+O` / (`space` at depth 0 only). Tree `b` and `S` stay left-only.
- `w` / `W` removes the focused linked worktree. Workspace, repo family, file, and hidden ignored rows do not remove. Confirm is a boxed overlay (`y` / `n`, merge status, `--force` when dirty). A non-linked row sets `Focus a linked worktree to remove` instead of a silent no-op. Bind-mount aliases remap the same way as in the TypeScript app. Ink uses the same keys
- `h` / `l` on a focused file diff pan long lines. They still fold when the tree is focused
- Stage / unstage / revert act on dirty files in the focused scope (file, dir, flat repo, or checkout). A file row stays single-file. A dir row writes the dir path and its children. Workspace, No-updates group, and family containers are a no-op. Hidden ignored stay out unless shown. Revert asks `y` / `Y` / `n` in a boxed overlay (tracked/untracked counts; `y` discards tracked and keeps untracked, except a sole untracked file which still deletes; `Y` also deletes untracked). Stage and unstage do not confirm
- `e` uses config `editor`, then `$EDITOR`, then `$VISUAL`, then `vim`. It opens a focused workspace dirty file, a focused commit-file row, or a focused commit-file diff. A TTY editor leaves the alternate screen and returns to the same fold, focus, and scroll. GUI editors (`cursor`, `code`) spawn without a remount. Resume drains leftover raw-mode keys
- `P` pushes the focused visible repo or checkout only when it is ahead, diverged, or has no upstream. In-sync is a no-op. Workspace never fans out. Hidden ignored stay out. Linked worktrees push only when that row is focused
- `S` on a dirty tree file or repo is create-only (`s stash`). `S` on a clean tree row that has `stash@{0}` is a no-op. `S` is left-pane only: a right-focused graph is a silent no-op. On a left-focused graph (commit drill) or when the menu is already open, a graph commit or uncommitted row offers apply / pop of the latest stash (`a` / `p`); drop stays off unless a graph stash row is focused. Graph stash rows offer apply / pop / drop of that `stash@{n}` (`a` / `p` / `D`, plus `S` on a left-focused graph stash). Never drop latest from a non-stash row. Menu `p` runs immediately. Drop asks `y` / `n` in a boxed overlay
- `d` switches visible targets that are off the default branch. Already-default is a no-op and does not pull. Dirty trees still skip
- `Enter` on a graph commit (or stash / uncommitted row) opens depth-1: left pane is the graph list, right pane is commit detail (short hash, refs, subject, author, relative date, plus the file tree). `Enter` on a file opens the commit diff. `Esc` pops diff to files (graph still left), then leaves the drill so the graph is the right pane again, then focuses the tree. Esc never quits. Hidden ignored repos stay out of the drill unless shown
- Graph stash rows are first-class: `a` apply, `p` pop, and `D` drop the focused `stash@{n}`. Drop asks `y` / `n` in a boxed overlay
- `b` on a tree checkout or flat repo opens the local branch picker. Type to filter. Enter checks out. `C` creates a branch at HEAD and checks it out. Selecting the current branch closes with `Already on …` and skips git. Dirty worktrees (`git diff` / `git diff --cached`, not untracked) refuse with `Dirty worktree — commit or stash first`. Local picker names never confirm against `origin/*`
- `b` on a focused graph commit checks out that commit's local and `origin/*` refs. One name checks out. Several names open a picker of those names only. Tags and other remotes stay out. A dirty tree refuses (`Dirty worktree — commit or stash first`). Selecting `origin/<name>` confirms only when a local branch of that name exists with a null or mismatched SHA; Yes checks out the local then `git merge --ff-only` of that already-fetched remote-tracking ref (no fetch, no reset, no pull). Scope is the checkout that owns the graph. Hidden ignored stay out
- `c` on a focused graph commit opens a name prompt and runs `git branch -- <name> <commitId>`. It does not check the new branch out. `c` is a no-op on a tree, file, or workspace row. Picker `C` still creates and checks out at HEAD
- Action / Effect loop: crossterm events become `Action`, dispatch updates state and returns an `Effect`. Focus / depth / kind gates live in `tui/gates.rs` (Ink `TREE_WRITE_BLOCKED_IDS` + `rightPaneLeftListAllowed`); pull/default kind lives in `ops.rs`. `r` is `ReloadRepo` for a focused checkout (or file / dir / flat repo) and `ReloadSnapshot` on the workspace row or No-updates group
- EasyMotion (`;` / Ctrl+Space) labels the currently visible rows on the focused list (tree, graph, or commit files). Prefix matching is partial / hit / miss. Hit jumps. Esc or a miss cancels and keeps the cursor. A focused diff does not start the overlay
- `T` cycles Tokyo Night → Monokai → Dracula → Gruvbox Dark → Catppuccin Mocha → Tokyo Night. Launch seed is `WS_STATUS_THEME`. The cycle stays in the current session; neither this TUI nor Ink writes a theme file
- Mouse is optional. Keys work without it. `m` toggles capture. Double-click is Enter. Click a fold chevron to toggle fold. Drag the tree / right splitter, or the in-diff RULE in split mode, to resize. `i` toggles inline / split without a mouse
- Tree / right and in-diff split ratios stay in the current session only. They reset on the next launch (Ink does not persist them either)
- `--plain` / `--json` / `-v` / `-p` / `-d` stay headless

Reviewed marks use `$XDG_STATE_HOME/my-workspace-status/viewed-files.json` (same identity and fingerprint as Ink). A mark drops when the file fingerprint changes. Space toggles dirty workspace-tree file rows at depth 0 only. The viewed glyph is Ink `ICON_VIEWED` from `icons.ts`: nerd nf-fa-eye `U+F06E` (``) / ASCII `*` — not `◉` and not a substitute eye. Cyan/blue, trailing before the status badge — not the clean check. Clean `ICON_CLEAN` (`` / `.`) paints only on the No updates group and on repo / checkout rows inside it.

## Tree chrome (Ink parity)

Daily tree paint matches Ink `src/tui/icons.ts` + `model/tree.ts` + `TreePane.tsx`:

- File rows: type glyph on the left, name, 2-column status badge on the **right** (`A` / `S` / `MS` / `M` / `D` / `R` / `U` / `C`). Untracked is `A`, staged-only is `S`, staged+unstaged is `MS`. Commit-file lists reuse the same chrome (name-status letters stay A/M/D/R/C, not workspace `S`).
- Folder / repo / branch / linked-worktree / sync / merge / viewed glyphs from the same registry. `WS_STATUS_GLYPHS=ascii` uses the same fallbacks (`#` `@` `L` `&` `/` `^` `v` `Y` `?` `=` `M` `o` `*`). Reviewed is Ink `ICON_VIEWED`: nerd nf-fa-eye `U+F06E` (``) / ASCII `*` — not `◉` and not a substitute eye. Ignored is `` / `~` (not `[ignored]`).
- Workspace header is file-oriented: `{cwd basename}` trailing `{N} changed · {ahead/behind/diverged/attention|all current}`.
- Linked worktrees under a family are checkout rows labeled by **branch** (Ink), not `wt <path>`. Detached linked checkouts fall back to the short worktree path. Linked-only snapshots (no primary in the window) stay a flat `Repo` row — no phantom primary container.
- Repo / checkout trailing sync marks match Ink (`↑N` / `↓N` / diverged / no-upstream). Merged-into-default / open-vs-default sit next to the branch. Up-to-date `` only inside No updates. The No updates count is a trailing number, not `(N)` in the label.

Rust extras stay: `q`, Tab, Home/End, family-row `b`, picker `C`. Confirms are boxed overlays (Ink `Confirm` / `StashDropConfirm` / `RemoveWorktreeConfirm` / `GraphCheckoutConfirm`), not status-line `y/n`. Bottom chrome matches Ink: mode pills, contextual hint chips (Rust extras `q` / `Tab` append and truncate with `…`), breadcrumb `workspace › [repo]`, armed search as a `/{query}` chip. Graph window/autoload, diff section headers, and stash meta are separate.

Glyphs live in `crates/workspace-status/src/tui/icons.rs`. Labels are built in `tree.rs` (`node_segments`); `render.rs` right-aligns trailing and paints the cursor bar. An empty workspace tree paints muted `No matching rows` (Ink TreePane). Commit-file lists do the same when loaded and empty; while git is still listing they paint `loading files…` (Ink CommitDetailPane).

## Optional Ink-only

The TypeScript Ink app stays in this repository, including its ink-testing e2e suite. That suite is optional. Daily-path TUI coverage for agents is `cargo test` (`crates/workspace-status/tests/tui_daily_e2e.rs`) on a TestBackend — no TTY.

See [tui-model.md](./tui-model.md) for the Ink keymap.

## Routing

`should_open_tui` is a pure function. Tests cover TTY vs flag decisions without a real TTY.

| Input | Effect |
| --- | --- |
| TTY, no headless flags | Ratatui TUI |
| `-i` / `--tui` | Ratatui TUI even when stdout is not a TTY |
| `--plain` or `--json` | Snapshot text / JSON (wins over `--tui`) |
| `-v`, `-p`, `-d` | Headless `--plain` path (with that flag; wins over `--tui`) |
| Non-TTY without `--tui` | Headless `--plain` unless `--json` |

## Layout

Left pane: workspace tree at depth 0. Clean default-branch repos sit under a folded `No updates` group. In a commit-files or commit-diff drill the left pane is the graph list so the graph stays visible.

Right pane: graph for a repo or worktree at depth 0, a numbered file diff when a dirty file is focused, or commit detail (meta header plus file tree) / that file's commit diff. Commit files still loading with 0 rows paint muted `loading files…`; a loaded empty list paints muted `No matching rows`.

A file diff paints a one-line path header (`{repo}/{path}  inline|split`, plus ` · full` when unlimited `-U` is on, ` · pan N` when panned) then STAGED / UNSTAGED / NEW section labels and a line-number gutter. After a commit drill the left pane is the graph, so that header is how the file path stays visible. Intra-line and syntax highlight stay Ink-only. Destructive confirms are boxed overlays with the same keys and copy as Ink.

Graph commits and stashes paint two lines, matching Ink layout A: subject on the node row; refs / `stash@{n}`, short hash, date (relative through 3 hours, then UTC `YYYY-MM-DD HH:MM`), and author on the spacer beneath. Narrow spacers keep whole ref chips and collapse the rest to `+N`; the footer lists every branch/tag (HEAD commit refs on the working-tree row). The gutter is capped like Ink (`≤30%` of the pane, ≥24 columns for refs+subject). PageUp/PageDown keep the focused row in view (`visible − 1` overlap). A 1-column position scrollbar tracks the graph list. The selected graph row uses Ink `▌` plus `cursorBg` (not reverse video). Local and `origin/*` names stay visible on commit spacers (matching local+remote tips merge into one chip). The spacer is not a second cursor, search, or EasyMotion target — `j` / `k` / Enter / Esc still treat the commit or stash as one item. When the pane is narrow, meta drops hash, then date, then author, keeping refs / `stash@{n}`.

Graph history matches Ink `gitLogGraphWindow`: `log --exclude=refs/stash --all --topo-order --date-order --skip --max-count` with a 300-commit window. Reaching the last loaded row fetches the next page and paints `loading older…`. Missing `stash^1` parents are loaded with `log --no-walk --ignore-missing` and sit after the log prefix so autoload skip stays on the window, not `commits.len()`. The working-tree row is always present on a loaded graph (`Working tree clean` / `Uncommitted changes`; dirty vs clean is the label only). The pane keeps a 2-line selection footer (footer before header when space is tight): uncommitted → HEAD commit ref chips, or `worktree · not a commit` when HEAD has none; spacer → `connector · not selectable`; stash → `stash@{n} ·` hash `·` date; commit → all ref chips or `(no refs)` `·` hash `·` author `·` date; empty → `no selection`. Subjects paint stronger than hash/date/author; branch chips, tag chips, HEAD, and the default branch use the Ink palette (`headMark` / `branchFeature` / `branchDefault` / `dir` / `modified`). Each gutter cell uses its `color_lane` from `DEFAULT_LANE_COLORS`. Linked worktree nodes use Ink `ICON_LINKED_WORKTREE` (`` / `L`), not the 2-column `🔗` emoji.

Rust extras that stay: `q`, Tab, Home/End, family-row `b`, picker `C`. The TypeScript Ink app is not removed.

The tree / right split defaults to 40% tree. The in-diff RULE defaults to 50/50. Drag either splitter (3-column grab band). Neither pane or column can collapse to zero. Both ratios are session-only.

Bottom chrome is two rows when idle (breadcrumb + status). `?` help replaces the status line, hides the breadcrumb, and shrinks the panes. Confirm / stash / create-branch / picker overlays do the same with Ink's row budget (breadcrumb stays). The breadcrumb mirrors the drill (`workspace › repo › hash`) and marks the right-focused segment with `[brackets]`. Trailing op-status / toasts sit on that row. While a multi-repo `f` / `p` / `P` / `d` (or the background fetch tick) is in flight, that slot shows `Fetching n/N…`, `Pulling n/N…`, `Pushing n/N…`, or `Switching n/N…` and the TUI repaints after each repo. The status line is Ink-style mode pills (`tree`/`flat`, `inline`/`split`), an armed-search `/{query}` chip, `? help` (or `z…` while a `zz` chord is pending), and contextual hint chips truncated with `…`. Rust extras `q` and `Tab` append after Ink hints. `/` typing replaces the bar with a SEARCH prompt; EasyMotion uses an EASY chip. Confirms are rounded boxed overlays (`y` / `Y` / `n`), not a status-line `y/n` prompt. `?` opens a 3-column overlay (MOVE / GIT / VIEW, same grouping and copy as Ink, with key chips). Rust extras `q`, Tab, picker `C`, stash `a p D`, and Home/End stay in those columns. Help `/` is query-append highlight-only: characters including `n`/`N` append; Esc clears the query; no Enter-arm and no next/prev wrap in help.

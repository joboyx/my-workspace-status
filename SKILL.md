---
name: my-workspace-status
description: Show status of all git repositories in current directory with AI-powered summary
triggers:
  - 'status'
  - 'git status'
  - 'repo status'
  - 'workspace status'
  - 'check status'
  - 'show status'
  - 'repository status'
  - 'ws'
source: 'joboyx/dotfiles'
---

You are helping the user review the status of all git repositories in the current directory.

**Your Task:**

1. Run `workspace-status` to gather repository information
2. Choose flags that match the user request
3. Show the CLI output directly unless the user explicitly asks for extra interpretation

**Instructions:**

- Run `workspace-status` (short alias: `ws`) from PATH
- **Always pass `--plain` or `--json` for agent runs.** On a TTY without one of those flags, the CLI opens an interactive TUI that waits for keyboard input and will hang the agent shell until killed. Do not rely on “non-TTY” alone — some harnesses allocate a TTY. Never use `-i` / `--tui` from an agent. `--plain` is the human renderer. `--json` prints the same workspace snapshot. See [docs/snapshot.md](./docs/snapshot.md).
- The CLI reads `.workspace-status-config.json` from the current workspace root; repos in `ignoredRepos` are skipped unless `-a` or `--all` is passed
- Optional `defaultBranches` map (repo path → branch) overrides the default branch for classification, markers, ordering, and `--default-branch` / TUI `d`; without an entry, defaults are derived as today
- `maxDepth` (default **3**) controls how many path segments below cwd are searched for git repos (so `acme/light-modules/*` is included by default)
- Optional `editor` for TUI `e` (same shape as `$EDITOR`). Omit the key or set `"editor": "vim"` for vim (the default). `"editor": "cursor"` opens Cursor IDE. Config overrides `$EDITOR` / `$VISUAL`.
- Pass one or more repo paths (e.g. `dotfiles`, `dotfiles notes`) to limit output to those repos; named repos are included even when listed in `ignoredRepos`
- The CLI already formats its own output, so prefer showing it as-is
- When changing the CLI or its output contract, keep `SAMPLE_OUTPUT.md` and `crates/workspace-status/tests/snapshot_contract.rs` in sync

**Output Format:**
The CLI produces:

1. **Summary section** (default output):
   - 📝 Changes: uncommitted, staged, both
   - 🔄 Sync status: behind, ahead, diverged
   - 🌿 Branches: any non-default branch (feature, bugfix, chore, release, unknown)
   - Shows ticket IDs extracted from branch names (e.g., ABCD-1234); otherwise shows `[branch]`

2. **Repository list** (only with `--verbose`):
   - Sorted by: clean repos on default branches first, then clean repos on non-default branches, then repos with changes
   - Clean default-branch repos are ordered by sync priority first, then default-branch priority, then repo name
   - Format: `<repo-name> <branch-emoji> <branch-name>  <sync-emoji>  <changes-emoji>  [optional-note]`
   - Symlinked directories and repos whose `.git` is a file (some submodules / non-dot linked checkouts) are discovered by the directory walk like regular repos
   - Linked worktrees under cwd (via `git worktree list`; walk still skips dot dirs such as `.worktrees/`) also appear, with a `🔗` repo prefix and optional `✅`/`🌱` merge-into-default marks on non-default branches; verbose column for dirty/clean state is **Files** (not Worktree)

**Emoji Legend:**

- Branch: 🔥 (main/master), 🚧 (feature), 🐛 (bugfix), 🔧 (chore), 🚀 (release), 🌿 (develop / unknown)
- Merge-into-default (non-default branches): ✅ merged, 🌱 open (omit when unknown / on default)
- Sync: ✅ (up-to-date), ⬇️ (behind), ⬆️ (ahead), 🔀 (diverged)
- Files (dirty/clean working tree): 💾 (clean), 📝 (uncommitted), ✨ (staged), ⚠️ (both staged+uncommitted)
- Linked checkout: 🔗 prefix on repo path; summary section `🔗 Linked worktrees`

**Optional Flags:**

- `[REPO...]`: Limit status to specific repos (relative to workspace root); bypasses `ignoredRepos` for named repos
- `--all`: Include repos listed in `.workspace-status-config.json` `ignoredRepos`
- `--fetch`: Fetch from remotes before checking status (slower but more accurate)
- `--verbose`: Show the aligned table before the summary
- `--pull`: Pull repos that are behind their upstream (auto-stashes dirty worktrees, then reapplies)
- `--default-branch`: Switch non-default branches to their default branch, then pull
- `--plain`: Force plain text report (required for agent runs unless `--json` — avoids interactive TUI hang)
- `--json`: Print the workspace snapshot as JSON (also disables the TUI)
- `--update`: Run the cargo-dist updater (`workspace-status-update`) and exit (never opens the TUI)
- `-i` / `--tui`: Force interactive TUI (humans only; never from agents)

**Interactive TUI (humans only):** On a TTY (without `-v` / `-p` / `-d` / `--plain` / `--json`), the CLI opens a ratatui TUI and blocks on keyboard input. Agents must always pass `--plain` or `--json` (see Instructions). Force TUI with `-i` / `--tui` only for interactive human use.

The TUI requires a **Nerd Font** — recommended `MesloLGM Nerd Font Mono` (the **Mono** variant; the proportional build breaks column alignment). In VS Code / Cursor set `terminal.integrated.fontFamily` in **User Settings**, since the terminal font is resolved on the client rather than in WSL. `WS_STATUS_GLYPHS=ascii` swaps in plain markers.

Defaults: directory tree (`t` toggles flat), split diff (`i` toggles inline), ignored repos hidden (`.` toggles; `-a` starts shown), and live refresh polling every 3s (`WS_STATUS_WATCH_MS=0` disables). Tree status letters are `A` added, `M` modified, `S` staged, `MS` staged+modified, `D` deleted, `R` renamed, `C` copied, `U` conflicted. The emoji legend above applies to the plain report only.

Live TUI keymap (full overlay detail in [docs/configuration.md](./docs/configuration.md); `W` / `w` is live even though that table omits it):

| Keys                                  | Action                                                                                                                                                 |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `j` / `k` · `↓` / `↑`                 | next / prev visible row (scrolls a focused diff)                                                                                                       |
| `l` / `→` · `h` / `←`                 | expand / collapse focused row (pan when right+diff)                                                                                                    |
| `z`                                   | toggle fold immediately; `zz` toggles subtree. No-op when the graph or a diff is focused                                                               |
| `gg` / `G`                            | first / last visible row on the focused list; on a focused **diff**, scroll to start / end                                                             |
| `t`                                   | flat ↔ directory tree (view-mode; works when right-focused)                                                                                            |
| `.`                                   | show / hide ignored repos (view-mode; works whether the app started with `-a`)                                                                         |
| `T`                                   | cycle built-in colour theme                                                                                                                            |
| Focus file                            | load diff (lazy)                                                                                                                                       |
| `s` / `u` / `x`                       | stage / unstage / revert (`x` confirms; untracked deletes from disk)                                                                                   |
| `S`                                   | stash menu (Shift+s; `s` stays stage)                                                                                                                  |
| `W` / `w`                             | remove linked worktree (confirm)                                                                                                                       |
| `i`                                   | inline ↔ split                                                                                                                                         |
| `r`                                   | refresh focused repo; full workspace if on workspace / “No updates” group                                                                              |
| `f`                                   | `git fetch` for the focused checkout, or primary checkouts on the workspace / family row. Linked worktrees only when that row is focused               |
| `p`                                   | pull behind primaries (workspace / family) or the focused checkout (including a linked worktree)                                                       |
| `P`                                   | push ahead\|diverged\|no-upstream (repo/checkout only; family uses the primary; first publish uses `-u`)                                               |
| `d`                                   | switch primaries (or the focused checkout) to the default branch and pull when clean; dirty checkouts skipped                                          |
| `e`                                   | open focused file in configured editor (default vim; config overrides `$EDITOR` / `$VISUAL`). vim leaves the alternate screen; Cursor stays mounted     |
| `space`                               | toggle reviewed on a dirty file row (eye mark). Stays until that file's diff or contents change. Non-file rows no-op (do not fold)                     |
| `b`                                   | depth 0: local branch picker. Graph commit: checkout local or `origin/*` (confirm when local is out of sync, then fast-forward to that `origin/*` ref) |
| `c`                                   | graph commit: create branch at commit (name overlay; ref only, no checkout)                                                                            |
| `m`                                   | graph commit: boxed confirm, then merge that ref into HEAD (`--ff-only`, else `--no-ff --no-edit`). Dirty tracked worktree refuses. Tree / stash / uncommitted `m` toggles mouse reporting |
| `Ctrl-o`                              | toggle full-file diff context (does not open editor)                                                                                                   |
| `/`                                   | search the focused pane (substring; does not hide rows). Enter arms the query; `Esc` clears                                                            |
| `n` / `N`                             | next / previous search match (only while a search is active; otherwise `p` is pull)                                                                    |
| `Ctrl-Space`                          | EasyMotion jump on the **focused** list (tree / graph / commit files). No-op on a focused diff                                                         |
| `?`                                   | keymap help overlay                                                                                                                                    |
| `PgUp` / `PgDn` / `Ctrl-u` / `Ctrl-d` | page the **focused** pane (list or diff)                                                                                                               |
| `m`                                   | toggle mouse reporting (on by default) except on a focused graph commit (merge, above)                                                                 |
| `Enter`                               | left: focus right; right: drill (stay on right). Double-click a list row runs Enter                                                                    |
| `q` / `Ctrl-C`                        | quit                                                                                                                                                   |
| `Esc`                                 | back (right → left, then pop depth). Never quits. Cancels overlays / chords first                                                                      |

When the right pane loads a new view, rows, or dataset (repo, pane kind, graph contents, or commit-file list key), it resets to the top — first selectable graph row, first commit-file row, or diff scroll 0. Same-view graph rebuilds (width/theme, autoload older) keep the cursor. `Ctrl-o` still keeps the hunk in view. The left tree still restores by id.

**Known limits:**

- macOS PageUp: the terminal must deliver the key (`Fn+Up` / mapped PageUp). Use `Ctrl-u` / `Ctrl-d` when PageUp does not arrive.
- `SIGKILL` skips alternate-screen restore (leave hooks do not run).
- Search Enter-to-arm: `/` opens a query; Enter arms it so `n` / `N` step matches. Esc clears.

**Your Task:**
Run `workspace-status` with the smallest set of flags that matches the request, **always including `--plain`**, then display the output directly. Only add your own summary when the user asks for analysis instead of raw output. Skipping `--plain` on a TTY can hang the shell on the interactive TUI.

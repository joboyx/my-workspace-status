# Diff rendering

`src/tui/diff/` plus `src/tui/DiffPane.tsx`.

The Rust TUI ports the same parse / row / header idea in
`crates/workspace-status/src/tui/diff.rs` (paint in `render.rs`). Path header,
line-number gutter, and STAGED / UNSTAGED / NEW labels match Ink. Intra-line
and syntax highlight stay Ink-only.

## Pipeline

```
git diff / git diff --cached      (src/git.ts)
        │
        ▼
parse.ts   parseUnifiedDiff(text) ──► Hunk[]  { header, lines: DiffLine[], oldStart, newStart }
        │
        ├─► rows.ts   buildDiffRows({staged, unstaged, mode, isNew}) ──► DiffRow[]   ← what the TUI renders
        │        inlineRows()      one cell per line
        │        sideBySideRows()  pairHunk() zips del-runs against add-runs by index
        │
        └─► parse.ts  buildDiffPaneLines() ──► string[]  via inline.ts / sideBySide.ts
                 plain-string renderers — currently exercised only by tests
        ▼
DiffPane.tsx  rowSegments() → cellSegments() → highlight.ts → Segment[] with raw = true
```

`parseUnifiedDiff` skips file-level headers (`diff --git`, `index`, `---`, `+++`) until the first `@@`, tracks 1-based `oldNo` / `newNo` per line, turns `\ No newline at end of file` into a `meta` line, and turns a `Binary files … differ` line into a single meta hunk with no header. Empty input returns `[]`, which is how "no diff" is detected upstream.

`gutterWidth(rows)` sizes the line-number column from the widest number present, minimum 2.

## Two renderers, one used

| Module | Output | Used by |
| --- | --- | --- |
| `rows.ts` | `DiffRow[]` (structured cells, line numbers, kinds) | `DiffPane.tsx` |
| `inline.ts` / `sideBySide.ts` via `buildDiffPaneLines` | `string[]` | tests only |

The string renderers were the original pane implementation and now serve as a plain-text contract for `test/tui-diff.test.ts`. They apply no colour and no syntax highlighting. Nothing in `src/` outside `parse.ts` imports them.

`sideBySide.ts` pads with `String.length`, not display width, so a line containing wide characters misaligns the column separator. `rows.ts` does not have this problem because it emits cells and lets the pane do the padding.

## Highlighting

`highlight.ts` wraps `cli-highlight` (Highlight.js). It is loaded with `createRequire` **on first use** so the plain report path never pays for it, and a failed load degrades to plain text rather than breaking the pane. `highlightLine(text, language)` caches by `` `${language} ${text}` `` up to `CACHE_LIMIT` (4000) entries, then clears wholesale.

Highlighting runs **per line**. Multi-line constructs — block comments, template literals, heredocs — are mis-tokenised as a result. That is a deliberate trade: cost stays proportional to what is on screen, which matters for large diffs.

The returned string is ANSI, carried through the `Segment` union as `raw: true`, which tells `Segments.tsx` to emit the text verbatim and apply no other style props.

### Current state — honest assessment

- **Only context and meta lines are highlighted.** `cellSegments` in `DiffPane.tsx` gives `add` / `del` cells a solid accent colour instead, on the reasoning that the change should be the loudest thing on the row. In practice most lines in a diff are added or deleted, so most of the pane is unhighlighted.
- **Inline and side-by-side share one path.** `cellSegments` highlights the full
  cell text, then truncates with `truncateAnsi`, so both modes see the same
  Highlight.js tokenisation (they may still cut at different column widths).
- **Padding uses plain truncated length.** Highlighted `raw` segments carry ANSI
  codes whose string length is not display width; padding still uses
  `truncateVisible(cell.text, codeWidth)` so column math stays stable.
- **`LANGUAGE_BY_EXTENSION` was expanded in Phase 4.** Existing entries remain; newly covered families include Vue/Svelte (`xml`), Ruby, PHP, Kotlin, Swift, C/C++, Terraform/HCL (`ini`), GraphQL, `.env` / dotenv (`bash`), ini/cfg/conf, Lua, R, Perl, Elixir, Erlang, Haskell, Clojure, Scala, Dart, and Vim. Bare filenames `dockerfile` / `makefile` are unchanged; unknown extensions still render plain.
- **Language is derived from `focusHint`**, which is `<repoPath>/<filePath>` — fine today because `languageForPath` only looks at the basename.

Phase 4 fixed highlight-before-truncate for ctx/meta; add/del cells still use solid accents.

## Untracked files — `newFile.ts`

An untracked file has no `git diff` output at all, so it is synthesised.

`useAppState.ts` orders it: after both `diffCachedFile` and `diffFile` come back empty *and* the node is untracked, it calls `readUntrackedAsDiff(abs, relPath)`. That reads the worktree file and returns:

| Condition | Result |
| --- | --- |
| Not a regular file, or read fails | `''` |
| Size > `HUGE_FILE_BYTES` (1 MB) | `Binary files /dev/null and b/<path> differ` stub |
| Buffer contains a NUL byte | same binary stub |
| Otherwise | `synthesizeAllAddDiff(text)` — a single `@@ -0,0 +1,N @@` hunk with every line prefixed `+` |

An empty file yields `@@ -0,0 +0,0 @@`. Both stub shapes are chosen so `parseUnifiedDiff` handles them without a special case.

`isNew` is set when the synthesised body is non-empty, and relabels the section header `NEW` instead of `UNSTAGED`.

## Cache invalidation

The diff cache in `useAppState.ts` is keyed `` `${repo}::${path}` `` and validated against `fileMtimeKey(abs)` — `` `${size}:${mtimeMs}` ``, or `missing`. The mtime key is checked *before* running git, so revisiting an unchanged file costs one `stat` rather than two `git diff` subprocesses. Any refresh that replaces snapshots clears the cache wholesale and bumps `diffEpoch` to force a reload.

Scroll position is reset only when `focusRepo`/`focusPath` change, so a live refresh of the file you are reading leaves you where you were.

## Side-by-side column drag

Split rows (`left + RULE + right`) take column widths from `diffSplit.sideBySideColumnWidths(paneWidth, fraction)`. Default fraction is 0.5 — the same `floor((width − 1) / 2)` as before. Mouse drag on the RULE (± 1 columns, same band as the tree/diff pane divider) updates a session-only `diffSplitFraction` in `App`; it is **not** written to disk or `SessionState`, so the next launch (and an editor remount) resets to 50/50, matching the pane split. Drag is armed only while `effectiveDiffMode` is side-by-side (`width ≥ NARROW_SXS`). `i` still toggles inline / split.

## Horizontal pan (Track D / D1)

Long diff lines are **not** word-wrapped. `SessionState.diffColOffset` is the horizontal column offset; `DiffPane` applies `codeWindow` / `sliceVisible` before highlight/truncate (`wrap="truncate"` only). When `focusPane === 'right'` and the right host shows a file diff, `h` / `←` and `l` / `→` emit `panDiff` (±1) instead of fold collapse/expand. Offset resets to 0 when the focused file changes. Header shows `· pan N` when offset > 0. Pan operates on plain text before highlight (context lines may clip mid-token).

## Full-file view

`ctrl+o` on a file row — or on a focused file **diff** — toggles that file's id into `SessionState.fullContext`.
While active, the lazy loader calls `diffFile` / `diffCachedFile` with
`FULL_DIFF_CONTEXT_LINES` (`-U999999`) so hunks expand to the whole file.
`Esc` or a second `ctrl+o` clears the flag for the focused file and reloads the
default-context diff. Cache keys are `repo::path` vs `repo::path::full`.
Untracked (`NEW`) files already synthesise full content and ignore this flag.

**Scroll-to-hunk (Track D / D2):** toggling full-file does **not** open an editor (`e` does). Before the reload, `anchorRowIndex` picks the topmost visible add/del line (else nearest hunk header at/above scroll). After the new rows load, `scrollToKeepRow` (upper-third preference) recenters the viewport on that anchor.

## Commit / stash / worktree diffs at depth 2 (JBY-037 P4)

At ViewStack depth 2 the right pane still uses `DiffPane` / `buildDiffRows`, but the loader in `useAppState` scopes content by `CommitFileSource`:

| Source | Staged slot | Unstaged slot |
| --- | --- | --- |
| `worktree` | `diffCachedFile` | `diffFile`, or `readUntrackedAsDiff` when both empty and untracked |
| `commit` | empty | `diffCommitFile` (`commit^`→`commit`, fallback `show --first-parent`) |
| `stash` | empty | `diffStashFile` (`diff <stash>^1 <stash> -- path`) |

`focusHint` / full-context (`ctrl+o`) follow the focused **commit-file** row id. Commit and stash diffs are single-sided by design — name-status letters land on `FileChange.unstagedStatus` so `statusLetterFromChange` keeps A/M/D/R (not workspace staged-only `S`).

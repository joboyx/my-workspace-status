# Diff rendering

`crates/workspace-status/src/tui/diff.rs` (paint in `tui/render.rs`). Path header,
line-number gutter, and STAGED / UNSTAGED / NEW labels. Intra-line word diff and
syntax highlighting are not implemented yet.

## Pipeline

```
git diff / git diff --cached      (git.rs git_diff_args)
        │
        ▼
parse_unified_diff(text) ──► Hunk[]  { header, lines, old_start, new_start }
        │
        ▼
build_diff_rows({staged, unstaged, mode, is_new}) ──► DiffRow[]
        inline: one cell per line
        side-by-side: pair del-runs against add-runs by index
        ▼
render.rs paints section headers, line-number gutter, and cells
```

`parse_unified_diff` skips file-level headers (`diff --git`, `index`, `---`, `+++`) until the first `@@`, tracks 1-based `old_no` / `new_no` per line, turns `\ No newline at end of file` into a `meta` line, and turns a `Binary files … differ` line into a single meta hunk with no header. Empty input returns no hunks, which is how "no diff" is detected upstream.

Gutter width sizes the line-number column from the widest number present, minimum 2.

## Highlighting

Intra-line word diff and syntax highlighting are not in this TUI yet. Add/del cells use a solid accent colour. Context and meta lines use the theme's muted / default text.

## Untracked files

An untracked file has no `git diff` output at all, so it is synthesised.

After both cached and worktree diffs come back empty *and* the node is untracked, the loader reads the worktree file:

| Condition | Result |
| --- | --- |
| Not a regular file, or read fails | empty |
| Size > `HUGE_FILE_BYTES` (1 MB) | `Binary files /dev/null and b/<path> differ` stub |
| Buffer contains a NUL byte | same binary stub |
| Otherwise | a single `@@ -0,0 +1,N @@` hunk with every line prefixed `+` |

An empty file yields `@@ -0,0 +0,0 @@`. Both stub shapes are chosen so the unified-diff parser handles them without a special case.

`is_new` is set when the synthesised body is non-empty, and relabels the section header `NEW` instead of `UNSTAGED`.

## Cache invalidation

The diff cache is keyed by repo + path and validated against `size:mtimeMs`, or `missing`. The mtime key is checked *before* running git, so revisiting an unchanged file costs one `stat` rather than two `git diff` subprocesses. Any refresh that replaces snapshots clears the cache.

Scroll position is reset only when the focused file changes, so a live refresh of the file you are reading leaves you where you were.

## Side-by-side column drag

Split rows (`left + RULE + right`) take column widths from `tui/split.rs`. Default fraction is 0.5. Mouse drag on the RULE (± 1 columns, same band as the tree/diff pane divider) updates a session-only split fraction; it is **not** written to disk, so the next launch resets to 50/50. Drag is armed only while the effective mode is side-by-side (`width ≥ NARROW_SXS`). `i` still toggles inline / split.

## Horizontal pan

Long diff lines are **not** word-wrapped. `h` / `←` and `l` / `→` pan when the right pane shows a file diff. Offset resets to 0 when the focused file changes. Header shows `· pan N` when offset > 0.

## Full-file view

`Ctrl+O` on a file row — or on a focused file **diff** — toggles unlimited unified context (`FULL_DIFF_CONTEXT_LINES`, `-U999999`) so hunks expand to the whole file. `Esc` or a second `Ctrl+O` restores the default-context diff. Untracked (`NEW`) files already synthesise full content and ignore this flag.

Toggling full-file does **not** open an editor (`e` does). After the new rows load, scroll recenters on the prior hunk/change anchor.

## Commit / stash / worktree diffs at depth 2

At drill depth 2 the right pane still uses the same `DiffRow` paint, and the left pane is the commit-file list (`j`/`k` there move files and load the focused file's diff). The loader scopes content by commit-file source:

| Source | Staged slot | Unstaged slot |
| --- | --- | --- |
| `worktree` | `git diff --cached` | `git diff`, or synthesised untracked when both empty |
| `commit` | empty | `diff_commit_file` (`commit^`→`commit`, fallback `show --first-parent`) |
| `stash` | empty | `diff_stash_file` (`diff <stash>^1 <stash> -- path`) |

`Ctrl+O` follows the focused **commit-file** row id. Commit and stash diffs are single-sided by design — name-status letters land on `FileChange.unstaged_status` so status letters keep A/M/D/R (not workspace staged-only `S`).

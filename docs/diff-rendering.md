# Diff rendering

`crates/workspace-status/src/tui/diff.rs` (paint in `tui/render.rs`, syntax in
`tui/syntax.rs`, word ranges in `tui/word_diff.rs`). Path header, line-number gutter, and
STAGED / UNSTAGED / NEW / COMMITTED / WORKING TREE labels. Changed words on a paired modified line get a
stronger background (see [Word highlight](#word-highlight)).

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
word_diff.rs (paint time, on a span-cache miss) ──► word ranges per paired del/add line
        ▼
render.rs paints section headers, line-number gutter, and cells

A compare tab uses `DiffContent::from_compare_lines`. The single section label is `COMMITTED`, not staged or unstaged. Header text is `<base-ref>...HEAD` (`abc1234^...abc1234` on a tab with a pinned head). `V` visual highlight paints on the compare diff the same way as on a Workspace file diff. `build_partial_patch` reads only that COMMITTED section for `PartialPatchKind::RevertCommitted` (compare `x` in a highlight, a reverse apply onto the worktree that restores the merge-base lines) and refuses a committed diff for every other kind. See [git-operations.md](./git-operations.md).

A commit-vs-working-tree compare tab uses `DiffContent::from_worktree_compare_lines` (`vs_worktree`). Its one section label is `WORKING TREE`: the new side is the file on disk. Header text is `abc1234 ↔ working tree`. `build_partial_patch` refuses that content for every kind.
```

`parse_unified_diff` skips file-level headers (`diff --git`, `index`, `---`, `+++`) until the first `@@`, tracks 1-based `old_no` / `new_no` per line, turns `\ No newline at end of file` into a `meta` line, and turns a `Binary files … differ` line into a single meta hunk with no header. Empty input returns no hunks, which is how "no diff" is detected upstream. A worktree `git diff` that fails keeps git's reason line in `DiffContent::error`, and the pane paints `git diff failed: <reason>` instead of `(no diff)`. When the other side loaded (staged ok, unstaged failed), `build_diff_rows` ends with a `DiffRow::Error` row carrying that text in the `deleted` colour.

Gutter width sizes the line-number column from the widest number present, minimum 2. A one-column comment mark sits to the left of that number column on every numbered cell, whether or not a comment exists, so adding a comment does not shift the numbers. Open comments paint `"` / nf-fa-comment. Resolved comments paint `'` / nf-fa-comment-o.
Line numbers and the gutter rule use the theme `muted` colour without DIM so they stay readable on a dark terminal.

## Display columns

Diff cells clip and pad by **display columns**, not Unicode scalar count. Clip uses `slice_cols` (meta / hunk) or `slice_styled_cols` (syntax spans). Pad uses `visible_width` of the clipped text (MesloLGS NF: emoji and other pictographs are two columns; private-use Nerd glyphs stay one). Hunk headers use the same clip. Emoji stay emoji. Do not replace them with ASCII placeholders. Char-count clip or pad under-counts those glyphs, so a split left cell grows and the in-diff RULE leaves the context-row column.

## Highlighting

Syntax highlighting uses `two-face` (syntect). Language comes from the file path (basename, then extension). A compare tab uses that tab's active file path, not a parked Workspace drill or `diff_path`. An unknown path uses Plain Text and the theme `repo` foreground. Tokens set **foreground** only. The file tab viewer's `syntax::highlight_file_window` shares this syntect setup (syntax set, theme map, `MAX_HIGHLIGHT_CHARS` budget) and feeds one stream from 200 lines above its window.

Add and del rows keep `palette.diffAddBg` / `palette.diffDelBg` on the gutter, sign, code, and pad. The `+` / `-` signs stay `added` / `deleted` (bold). Cursor, visual-line, and unfocused selected overlays tint that row background under the code and pad (see word highlight below); the gutter and sign take the flat cursor bar and keep their colours. A search overlay replaces the row background. A syntax colour that fails a contrast floor of 3.0 against the add/del background falls back to `repo`. Syntect parse state is kept across lines inside one hunk (old-file vs new-file streams) so a JSON property line after `{` still gets token colours. A section label or hunk header starts a new pair of highlighters. Paint highlights the visible viewport. Lines above the viewport in the same hunk still feed parse state. Paint reuses the last span list when the file path, theme, viewport, and diff text stay the same. A watch or compare reload that replaces same-length text still computes new spans.

Context lines use the same token foregrounds on the surface (no add/del tint). Meta lines stay muted. Line numbers use muted without DIM.

## Word highlight

Every `draw_diff_pane` caller (workspace dirty files, compare tabs, commit and stash drills) paints it. A modified line keeps its row background (`palette.diffAddBg` / `palette.diffDelBg`). The words that changed on that line get a stronger shade of the same hue: `palette.diffAddWordBg` on the added line, `palette.diffDelWordBg` on the removed line. Only the code column changes. Line numbers, the comment mark, the gutter rule, and the `+` / `-` sign stay on the row background. Signs stay `added` / `deleted` (bold).

Git output stays line-based (no `--word-diff`). The parser, hunk headers, line numbers, and partial patch (`V` highlight, stage / unstage / revert) do not change. `tui/word_diff.rs` computes word ranges from the paired line texts as a paint overlay.

Pairing is the side-by-side pairing (del-runs against add-runs by index), in inline mode as well as split. Only a paired line that shares some unchanged non-whitespace text with its pair gets word highlight. These keep the row background only:

- unpaired lines: pure add, pure del, untracked `NEW`, the extra lines of a longer run, a line split from its run by a `\ No newline` meta line
- a total rewrite (no shared text)
- a line over 4096 chars, or a pair over the fixed work budget

Tokens are words (letters, digits, `_`), whitespace runs, and each other char (punctuation, symbol, emoji) on its own. Combining marks, VS16, ZWJ, and skin-tone modifiers stay with their glyph. A change inside a word highlights the whole word. Several edits paint several spans; unchanged text between them stays on the row background. A whitespace-only edit highlights the changed whitespace.

Syntax foregrounds stay; word highlight sets the background only. The 3.0 contrast floor is checked against the background behind the glyph (word or row background). A failing colour falls back to `repo`. Cursor, visual-line, and unfocused selected rows tint the add/del code and word backgrounds: each takes 3/4 of the theme's cursor shift (cursor bar minus surface). Word spans stay stronger than the row, and syntax foregrounds are checked again against the 3.0 floor on the tinted background. The gutter, sign, and context, meta, and empty cells keep the flat cursor bar. A search overlay still replaces the whole row background, word spans included.

Unwrapped: spans clip and pan with the line by display columns and stay on the changed glyphs. Emoji are not split. Wrapped: continuation rows keep the word highlight on their slice of the line. `j` / `k` still move by logical row.

Word ranges are computed with the syntax spans on a cache miss and live in the same span cache. A repaint with the same text reuses them. A watch or compare reload that replaces same-length text computes them again.

## Untracked files

An untracked file has no `git diff` output at all, so it is synthesised.

After both cached and worktree diffs come back empty *and* the node is untracked, the loader reads the worktree file:

| Condition | Result |
| --- | --- |
| Not a regular file, or read fails | empty |
| Size > `HUGE_FILE_BYTES` (1 MB), NUL in the first 8000 bytes | `Binary files /dev/null and b/<path> differ` stub |
| Size > `HUGE_FILE_BYTES` (1 MB), no NUL there | `too large to preview (N.N MB)` stub |
| Buffer contains a NUL byte | same binary stub |
| Otherwise | a single `@@ -0,0 +1,N @@` hunk with every line prefixed `+` |

An empty file yields `@@ -0,0 +0,0 @@`. The parser treats both stub lines as one header-less meta hunk, like `Binary files … differ`.

`is_new` is set when the synthesised body is non-empty, and relabels the section header `NEW` instead of `UNSTAGED`.

## Cache invalidation

The diff cache is keyed by repo + path and validated against `size:mtimeMs`, or `missing`. The mtime key is checked *before* running git, so revisiting an unchanged file costs one `stat` rather than two `git diff` subprocesses. Any refresh that replaces snapshots clears the cache.

Scroll position is reset only when the painted file-diff identity changes, so a live refresh of the file you are reading leaves you where you were. Workspace diffs key that identity on `repo`+`path`. Depth-2 commit diffs key it on `repo`+source+`path`. A workspace file and a commit file that share a path are different views.

## Side-by-side column drag

Split rows (`left + RULE + right`) take column widths from `tui/split.rs`. Default fraction is 0.5. Mouse drag on the RULE (± 1 columns; the hit test reads the right pane's painted content column, so boxed and flat panes map the same) updates a session-only split fraction; it is **not** written to disk, so the next launch resets to 50/50. Drag is armed only while the diff paints side-by-side (`diff_pane_mode`): the painted width (`diff_paint_width`, the right pane less its 1-column scrollbar column, reserved whether or not the bar shows) is ≥ `NARROW_SXS` (100), so the right pane needs ≥ 101 columns. `i` still toggles inline / split.

## Path header

The path header sits above the diff body in `draw_diff_pane`. A path wider than the pane wraps over several rows by display columns and can break anywhere. `diff_pane_header_rows` sets the row count: the rows the path needs, at most half the pane height (at least 1). A path cut by that cap is clipped on its last row. The muted extras (`inline|split`, ` · full`, ` · wrap`, ` · pan N`, `shown/total`) follow the path on its last row and clip at the pane edge. The row count does not depend on the extras or the scroll position, so the body does not move while it scrolls or pans. Paint, click, PageUp / PageDown, and the viewport use the same count.

## Focused row

A focused file-diff row (section, hunk, or line) paints the same cursor bar as other lists. An unfocused file-diff still marks that row with the thinner inactive marker and `cursorBgInactive`. `j` / `k`, PageUp / PageDown, Ctrl-u / Ctrl-d, click, search, and vertical wheel move that row. The viewport keeps it near the vertical middle (`list_viewport_start`, same helper as the workspace tree). `gg` / `G` and Home / End jump to the first / last row.

## Row to source line

`row_line_ref(content, mode, row)` gives the source line behind a painted row: section, kind (add / del / context), and old / new line numbers. It walks the same row list as `build_diff_rows`, so `row` is the diff cursor. A split row that pairs a deleted and an added line gives the added (new-side) line; a row with only a deleted line gives that line. Section labels, hunk headers, `\ No newline` and binary markers, and error rows give none. The file path comes from the caller.

## Current-line blame

With line blame on (`B`, `viewDefaults.lineBlame`), the focused line ends with a dimmed (`palette.muted`) note: `{author}, {age} · {sha7} · {subject}`. Rules:

- Only the focused row of a focused diff pane, or the cursor line of a file tab. A row the cursor is not on, and a diff whose pane does not have focus, paint none.
- It paints as `"  " + text` inside the row's trailing blank pad and keeps the pad's width, so row heights, the gutter, wrap, and the pan range do not change. With wrap on it goes on the last wrap row of the cell. No pad (the code fills the width or is panned across it) or fewer than 12 free columns paints none. A cut text ends in `…`; the subject is cut first.
- Split mode paints it in the blamed side's cell: the new (right) cell for an added or context line, the old (left) cell for a deleted line.
- An added line in UNSTAGED reads `You · uncommitted`, and in STAGED `You · staged`, with no git call. NEW (untracked), binary, meta, hunk, and error rows paint none. While git runs, and when git has no blame for the line, the row paints none.
- On an add/del row the note and the pad after it sit on the flat cursor bar, not on the tinted row background, so the muted text stays readable.
- No note paints while a mouse drag selection is active, because release copies the painted screen cells.

`AppState::focused_line_annotation` (`tui/state/line_blame.rs`) picks the text; `render.rs` `put_line_annotation` paints it.

## Soft wrap

`\` toggles soft word-wrap on any file-diff body that uses `draw_diff_pane` (workspace dirty files, compare tabs, commit / stash drills). Wrap is on by default. `viewDefaults.wrap` in `.workspace-status-config.json` (`wrap` or `unwrap`) sets the launch state, applied once at TUI start (see [configuration.md](./configuration.md)). The toggle changes the current session only: it lives on `AppState.diff_wrap` and nothing writes the config file. Status toasts `wrap on` / `wrap off`. The path header adds ` · wrap` while wrap is on.

Wrap uses **display columns** (same Meslo `visible_width` rules as clip), not Unicode scalar count. Long lines wrap inside the code column. Line numbers, the comment mark, the gutter rule, and the `+` / `-` sign paint on the first visual row of a logical line. Continuation rows keep the same add/del background and a blank gutter / sign so the code column stays aligned. `j` / `k` still move by logical row. Side-by-side rows wrap each cell and take the taller side.

While wrap is on, horizontal pan is a no-op: `h` / `l`, Shift-arrows, and mouse/trackpad hscroll do not change `diff_col_offset`, the header does not show `· pan N`, and the 1-row horizontal bar stays hidden. Turning wrap on resets pan to 0. Turning wrap off restores clip + pan.

## Horizontal pan

When wrap is off, long diff lines clip to the pane. The cursor bar uses one column. Pan max uses the remaining content width so the last character stays reachable. `h` / `←` and `l` / `→` pan when the right pane shows a file diff (same keys pan a focused graph or commit-file list). Shift-Left / Shift-Right pan the focused pane, including the tree. Mouse horizontal wheel (and Shift-wheel) pans the pane under the pointer without moving the focused row. When a file diff has long lines, trackpad hscroll (SGR `66`/`67`, same `tui/tty.rs` decode as the live loop) over the left pane pans that diff rather than a short tree label. Offset resets to 0 when the painted file-diff identity changes. Header shows `· pan N` when offset > 0. A 1-row horizontal bar paints after the view leaves the left edge (`diff_col_offset` > 0, like the graph bar), so an unpanned diff keeps its last body row; a 1-column vertical bar paints whenever the diff overflows the pane, at the top too. Rows always leave that column (`diff_paint_width`), so split, wrap, and pan widths do not change when the bar shows. The horizontal thumb is draggable (`hit_split` / `SplitDrag::DiffHScrollbar`, same stack as the graph bars). A track click jumps `diff_col_offset` without moving `diff_cursor`. Compare tabs use this same `draw_diff` paint. Tree and commit-file lists do not paint an h-bar. Rows stay clipped to the pane width.

## Full-file view

`Ctrl-o` on a file row — or on a focused file **diff** — toggles unlimited unified context (`FULL_DIFF_CONTEXT_LINES`, `-U999999`) so hunks expand to the whole file. `Esc` or a second `Ctrl-o` restores the default-context diff. Untracked (`NEW`) files already synthesise full content and ignore this flag.

Toggling full-file does **not** open an editor (`e` does). After the new rows load, scroll recenters on the prior hunk/change anchor.

## Commit / stash / worktree diffs at depth 2

At drill depth 2 the right pane still uses the same `DiffRow` paint, and the left pane is the commit-file list (`j`/`k` there move files and load the focused file's diff; a folder row shows the [folder summary](#folder-summary)). The loader scopes content by commit-file source:

| Source | Staged slot | Unstaged slot |
| --- | --- | --- |
| `worktree` | `git diff --cached` | `git diff`, or synthesised untracked when both empty |
| `commit` | empty | `diff_commit_file` (`commit^`→`commit`, fallback `show --first-parent`) |
| `stash` | empty | `diff_stash_file` (`diff <stash>^1 <stash> -- path`) |

`Ctrl-o` follows the focused **commit-file** row id. Commit and stash diffs are single-sided by design — name-status letters land on `FileChange.unstaged_status` so status letters keep A/M/D/R (not workspace staged-only `S`).

## Folder summary

On a depth-2 drill or a compare tab, a focused folder row in the file list paints a folder summary in the right pane instead of a diff (`draw_folder_summary` in `tui/render.rs`). The summary comes from `AppState::folder_summary`, built in memory from the loaded file list. It runs no git.

- Header: `dir/` in the heading style, then muted `N files · +A −D`. The header wraps like the path header. The totals drop out when no file under the folder has line counts.
- One row per changed file under the folder, at any depth, sorted by path. Each row uses the flat-mode file-list segments (icon, name, dimmed folder, status badge). `+added` (added colour) and `−deleted` (deleted colour) sit before the badge. A file with no counts (binary, untracked, failed numstat) shows the badge only.
- ASCII glyph mode paints `-` for deletions.
- Rows past the pane height fold into a last row `… N more`. The summary does not scroll.
- The status row drops the `split` / `inline` pill, and the side-by-side rule is not armed for drag.

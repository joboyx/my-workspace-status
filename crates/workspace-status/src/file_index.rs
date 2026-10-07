//! Quick Open file listing, fuzzy scoring, and the plain-text file read.
//!
//! The index lists the files of one or more checkouts from the workspace
//! snapshot: tracked plus untracked files, minus ignored ones. It asks
//! `git ls-files` instead of walking the tree, so `.gitignore`,
//! `.git/info/exclude`, and `core.excludesFile` apply exactly as git applies
//! them, and no ignore engine ships in this crate. `.git`, `target`, and
//! `node_modules` components are dropped even when tracked.
//!
//! Caps: at most [`MAX_INDEX_ENTRIES`] listed files (the index then sets
//! `truncated`), [`MAX_RESULTS`] scored hits per query, and
//! [`MAX_FILE_BYTES`] for a file read.
//!
//! A directory that is not a git checkout lists nothing. The snapshot only
//! holds git checkouts, so a failed `ls-files` means the checkout vanished
//! or broke after the snapshot. That root contributes no rows and its
//! error goes to [`FileIndex::errors`].
//!
//! Every function here does blocking I/O or a full scan. Call them off the
//! TUI draw/event thread.

use std::io::Read;
use std::path::Path;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::git;
use crate::helpers::visible_width;

/// Most listed files one index keeps before it sets [`FileIndex::truncated`].
pub const MAX_INDEX_ENTRIES: usize = 200_000;

/// Most hits one Quick Open query returns.
pub const MAX_RESULTS: usize = 200;

/// Largest file [`read_text_file`] loads for the file tab (2 MiB).
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Bytes [`read_text_file`] checks for a NUL before it calls a file binary.
///
/// Search in files skips a file by the same rule.
pub(crate) const BINARY_SNIFF_BYTES: usize = 8000;

/// Tab stop width [`read_text_file`] expands `\t` to.
const TAB_WIDTH: usize = 4;

/// Path components never listed, even when git tracks them.
const SKIPPED_COMPONENTS: [&str; 3] = [".git", "target", "node_modules"];

/// One checkout the index lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexRoot {
    /// Snapshot `repo` path of the checkout, relative to the workspace cwd.
    pub checkout: String,
    /// Display prefix for this root's files: `""`, or `"<checkout>/"` (the
    /// snapshot `repo` path, unique per checkout) when the index spans
    /// several checkouts.
    pub prefix: String,
}

/// One listed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Index into [`FileIndex::roots`].
    pub root: u32,
    /// Root prefix plus the checkout-relative path. Scored and painted.
    pub display: String,
    /// Byte offset in `display` where the checkout-relative path starts.
    pub rel_start: u32,
}

impl FileEntry {
    /// Path relative to the entry's checkout (`display` minus the prefix).
    pub fn rel(&self) -> &str {
        &self.display[self.rel_start as usize..]
    }
}

/// Files listed for a Quick Open scope, sorted by `display`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileIndex {
    /// Checkouts this index lists, in request order.
    pub roots: Vec<IndexRoot>,
    /// Listed files, sorted by `display`, deduplicated per root.
    pub entries: Vec<FileEntry>,
    /// True when listing stopped at the entry cap.
    pub truncated: bool,
    /// One `<checkout>: <reason>` line per root whose listing failed.
    pub errors: Vec<String>,
}

impl FileIndex {
    /// Snapshot checkout path (relative to cwd) that `entry` belongs to.
    pub fn checkout(&self, entry: &FileEntry) -> &str {
        &self.roots[entry.root as usize].checkout
    }
}

/// Whether a listed checkout-relative path stays in the index.
///
/// False when any `/` component is `.git`, `target`, or `node_modules`:
/// build output and vendored packages drown real hits, even when a repo
/// tracks them.
pub fn keep_listed_path(rel: &str) -> bool {
    !rel.split('/')
        .any(|part| SKIPPED_COMPONENTS.contains(&part))
}

/// List the files of every root under `cwd`, at most `max_entries`.
///
/// Per root: `git ls-files` ([`git::list_checkout_files`]), then
/// [`keep_listed_path`], then only paths that exist and are not a
/// directory (`symlink_metadata`). That drops deleted-but-indexed files and
/// submodule directories. Listing stops at `max_entries` and sets
/// `truncated`. Entries are then sorted by `display` (root order on a tie)
/// and deduplicated on `(root, display)`: an unmerged path repeats once
/// per index stage, while two roots that show the same `display` both stay.
pub fn build_file_index(cwd: &Path, roots: Vec<IndexRoot>, max_entries: usize) -> FileIndex {
    let mut index = FileIndex {
        roots,
        ..FileIndex::default()
    };
    'roots: for (root_idx, root) in index.roots.iter().enumerate() {
        let checkout_abs = cwd.join(&root.checkout);
        let listed = match git::list_checkout_files(&checkout_abs) {
            Ok(listed) => listed,
            Err(err) => {
                let label = if root.checkout.is_empty() {
                    "."
                } else {
                    root.checkout.as_str()
                };
                index.errors.push(format!("{label}: {err}"));
                continue;
            }
        };
        for rel in listed {
            if !keep_listed_path(&rel) {
                continue;
            }
            match std::fs::symlink_metadata(checkout_abs.join(&rel)) {
                Ok(meta) if !meta.is_dir() => {}
                _ => continue,
            }
            if index.entries.len() >= max_entries {
                index.truncated = true;
                break 'roots;
            }
            index.entries.push(FileEntry {
                root: root_idx as u32,
                display: format!("{}{rel}", root.prefix),
                rel_start: root.prefix.len() as u32,
            });
        }
    }
    index
        .entries
        .sort_by(|a, b| a.display.cmp(&b.display).then(a.root.cmp(&b.root)));
    index
        .entries
        .dedup_by(|a, b| a.root == b.root && a.display == b.display);
    index
}

/// One scored Quick Open hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHit {
    /// Index into [`FileIndex::entries`].
    pub entry: usize,
    /// Fuzzy score; higher ranks first. `0` for a blank query.
    pub score: u32,
    /// Char indices into `display` (`display.chars()` positions) of the
    /// matched characters, sorted and deduplicated. Empty for a blank query.
    pub indices: Vec<u32>,
}

/// Rank `index` entries against `query`, best first, at most `limit` hits.
///
/// A blank query returns the first `limit` entries in index order with
/// score 0 and no indices. Otherwise each whitespace-separated word is a
/// fuzzy atom (`nucleo-matcher` with path bonuses, smart case: an
/// uppercase letter makes that word case-sensitive). Ties break on the
/// shorter `display`, then `display` order. Only the best `limit` hits are
/// fully sorted, and match indices are computed for those hits only.
pub fn score_files(index: &FileIndex, query: &str, limit: usize) -> Vec<FileHit> {
    if query.trim().is_empty() {
        return (0..index.entries.len().min(limit))
            .map(|entry| FileHit {
                entry,
                score: 0,
                indices: Vec::new(),
            })
            .collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize)> = index
        .entries
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            pattern
                .score(Utf32Str::new(&entry.display, &mut buf), &mut matcher)
                .map(|score| (score, idx))
        })
        .collect();
    let rank = |(score_a, a): &(u32, usize), (score_b, b): &(u32, usize)| {
        score_b.cmp(score_a).then_with(|| {
            let (a, b) = (&index.entries[*a].display, &index.entries[*b].display);
            a.len().cmp(&b.len()).then_with(|| a.cmp(b))
        })
    };
    if limit < scored.len() {
        scored.select_nth_unstable_by(limit, rank);
        scored.truncate(limit);
    }
    scored.sort_unstable_by(rank);
    let mut chars: Vec<char> = Vec::new();
    scored
        .into_iter()
        .map(|(score, entry)| {
            // Indices come from a per-char haystack, not `Utf32Str::new`
            // (which folds a grapheme cluster into one position), so they
            // line up with `display.chars()`.
            let display = &index.entries[entry].display;
            let haystack = if display.is_ascii() {
                Utf32Str::Ascii(display.as_bytes())
            } else {
                chars.clear();
                chars.extend(display.chars());
                Utf32Str::Unicode(&chars)
            };
            let mut indices = Vec::new();
            pattern.indices(haystack, &mut matcher, &mut indices);
            indices.sort_unstable();
            indices.dedup();
            FileHit {
                entry,
                score,
                indices,
            }
        })
        .collect()
}

/// Outcome of [`read_text_file`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileRead {
    /// Paintable lines, plus the widest line in terminal columns.
    Text {
        /// Lines without their `\n` / `\r\n`, tabs expanded, control
        /// characters replaced with U+FFFD.
        lines: Vec<String>,
        /// Widest line by [`visible_width`].
        max_cols: usize,
    },
    /// A NUL byte in the first 8000 bytes.
    Binary,
    /// Larger than the `max_bytes` cap.
    TooLarge {
        /// File size on disk.
        bytes: u64,
    },
    /// Not a regular file, or a metadata / read error, as text.
    Failed(String),
}

/// Read `path` as plain text for the file tab, refusing files over `max_bytes`.
///
/// Only a regular file (after following symlinks) is read, and never more
/// than `max_bytes + 1` bytes, so a device, FIFO, or growing file cannot
/// block the caller or exhaust memory.
///
/// A NUL in the first 8000 bytes means binary. Invalid UTF-8 is replaced
/// lossily. Lines split on `\n` with a trailing `\r` stripped; a final
/// newline does not add an empty last line. `\t` expands with spaces to
/// the next multiple of 4 columns; other control characters become U+FFFD
/// so they cannot drive the terminal.
pub fn read_text_file(path: &Path, max_bytes: u64) -> FileRead {
    // `metadata` follows symlinks. Check the target before `open`: opening
    // a FIFO blocks, and a device such as `/dev/zero` reports length 0 but
    // never ends.
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) => return FileRead::Failed(err.to_string()),
    };
    if !meta.is_file() {
        return FileRead::Failed("not a regular file".into());
    }
    if meta.len() > max_bytes {
        return FileRead::TooLarge { bytes: meta.len() };
    }
    let mut raw = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|file| file.take(max_bytes.saturating_add(1)).read_to_end(&mut raw));
    if let Err(err) = read {
        return FileRead::Failed(err.to_string());
    }
    // The file grew after the metadata call.
    if raw.len() as u64 > max_bytes {
        return FileRead::TooLarge {
            bytes: meta.len().max(raw.len() as u64),
        };
    }
    if raw[..raw.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return FileRead::Binary;
    }
    let text = String::from_utf8_lossy(&raw);
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let lines: Vec<String> = body
        .split('\n')
        .map(|line| paintable_line(line.strip_suffix('\r').unwrap_or(line)))
        .collect();
    let max_cols = lines.iter().map(|l| visible_width(l)).max().unwrap_or(0);
    FileRead::Text { lines, max_cols }
}

/// Expand tabs and replace control characters in one line.
fn paintable_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut col = 0usize;
    for ch in line.chars() {
        push_paintable_char(&mut out, &mut col, ch);
    }
    out
}

/// Append one source character to a paintable line, the way the file tab
/// paints it.
///
/// `col` is the terminal column `out` ends at; it advances by what was
/// appended. `\t` becomes spaces up to the next multiple of 4 columns, and
/// any other control character becomes U+FFFD. Search in files builds its
/// hit lines char by char through this, so a hit line and the file tab line
/// read the same.
pub(crate) fn push_paintable_char(out: &mut String, col: &mut usize, ch: char) {
    if ch == '\t' {
        let pad = TAB_WIDTH - *col % TAB_WIDTH;
        out.extend(std::iter::repeat_n(' ', pad));
        *col += pad;
        return;
    }
    let ch = if ch.is_control() { '\u{FFFD}' } else { ch };
    let mut utf8 = [0u8; 4];
    out.push(ch);
    *col += visible_width(ch.encode_utf8(&mut utf8));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{git, init_repo, unique_dir};
    use std::fs;
    use std::path::PathBuf;

    fn root(checkout: &str, prefix: &str) -> IndexRoot {
        IndexRoot {
            checkout: checkout.to_string(),
            prefix: prefix.to_string(),
        }
    }

    /// Workspace dir with one repo at `<cwd>/app`.
    fn workspace_with_app(tag: &str) -> (PathBuf, PathBuf) {
        let cwd = unique_dir(&format!("ws-file-index-{tag}"));
        let app = cwd.join("app");
        init_repo(&app);
        (cwd, app)
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, body).expect("write fixture");
    }

    fn displays(index: &FileIndex) -> Vec<&str> {
        index.entries.iter().map(|e| e.display.as_str()).collect()
    }

    /// An index over `paths` with no git behind it (scorer tests).
    fn index_of(paths: &[&str]) -> FileIndex {
        let mut entries: Vec<FileEntry> = paths
            .iter()
            .map(|p| FileEntry {
                root: 0,
                display: p.to_string(),
                rel_start: 0,
            })
            .collect();
        entries.sort_by(|a, b| a.display.cmp(&b.display));
        FileIndex {
            roots: vec![root(".", "")],
            entries,
            truncated: false,
            errors: Vec::new(),
        }
    }

    fn hit_displays<'a>(index: &'a FileIndex, query: &str) -> Vec<&'a str> {
        score_files(index, query, MAX_RESULTS)
            .iter()
            .map(|h| index.entries[h.entry].display.as_str())
            .collect()
    }

    /// Score of `display` for `query`; panics when it does not match.
    fn score_of(index: &FileIndex, query: &str, display: &str) -> u32 {
        score_files(index, query, MAX_RESULTS)
            .iter()
            .find(|h| index.entries[h.entry].display == display)
            .unwrap_or_else(|| panic!("{display} does not match {query:?}"))
            .score
    }

    /// `better` scores strictly above `worse` (not a length tie-break) and
    /// ranks first.
    fn assert_outranks(paths: &[&str], query: &str, better: &str, worse: &str) {
        let index = index_of(paths);
        let (b, w) = (
            score_of(&index, query, better),
            score_of(&index, query, worse),
        );
        assert!(b > w, "{query:?}: {better} {b} vs {worse} {w}");
        assert_eq!(hit_displays(&index, query)[0], better);
    }

    #[test]
    fn ls_files_respects_gitignore_and_skips_target_node_modules_git() {
        let (cwd, app) = workspace_with_app("ignore");
        write(&app, ".gitignore", "*.log\n");
        write(&app, "a.log", "log\n");
        write(&app, "src/main.rs", "fn main() {}\n");
        write(&app, "target/x.rs", "// build output\n");
        git(&app, &["add", ".gitignore", "src/main.rs"]);
        git(&app, &["add", "-f", "target/x.rs"]);
        git(&app, &["commit", "-q", "-m", "files"]);
        write(&app, "new.txt", "untracked\n");
        write(&app, "node_modules/m.js", "vendored\n");
        write(&app, "pkg/node_modules/n.js", "nested vendored\n");
        write(&app, "crates/a/target/y.rs", "nested build output\n");
        write(&app, "src/target.rs", "a file named target\n");

        // git itself drops the trees (pathspec), before the backstop filter.
        let raw = git::list_checkout_files(&app).expect("ls-files");
        for gone in [
            "a.log",
            "target/x.rs",
            "node_modules/m.js",
            "pkg/node_modules/n.js",
            "crates/a/target/y.rs",
        ] {
            assert!(!raw.iter().any(|p| p == gone), "{gone} listed in {raw:?}");
        }
        for want in ["src/main.rs", "src/target.rs", "new.txt", "README.md"] {
            assert!(raw.iter().any(|p| p == want), "{want} missing from {raw:?}");
        }

        let index = build_file_index(&cwd, vec![root("app", "")], MAX_INDEX_ENTRIES);
        let shown = displays(&index);
        for want in ["src/main.rs", "new.txt", "README.md"] {
            assert!(shown.contains(&want), "{want} missing from {shown:?}");
        }
        for gone in ["a.log", "target/x.rs", "node_modules/m.js"] {
            assert!(!shown.contains(&gone), "{gone} listed in {shown:?}");
        }
        assert!(
            shown.iter().all(|d| !d.split('/').any(|c| c == ".git")),
            "{shown:?}"
        );
        assert!(index.errors.is_empty(), "{:?}", index.errors);
        assert!(!index.truncated);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn deleted_tracked_file_is_dropped() {
        let (cwd, app) = workspace_with_app("deleted");
        write(&app, "gone.txt", "bye\n");
        git(&app, &["add", "gone.txt"]);
        git(&app, &["commit", "-q", "-m", "gone"]);
        fs::remove_file(app.join("gone.txt")).expect("rm");

        let index = build_file_index(&cwd, vec![root("app", "")], MAX_INDEX_ENTRIES);
        assert_eq!(displays(&index), vec!["README.md"]);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn workspace_roots_prefix_display_and_keep_rel() {
        let cwd = unique_dir("ws-file-index-roots");
        init_repo(&cwd.join("app"));
        init_repo(&cwd.join("lib"));

        let index = build_file_index(
            &cwd,
            vec![root("app", "app/"), root("lib", "lib/")],
            MAX_INDEX_ENTRIES,
        );
        assert_eq!(displays(&index), vec!["app/README.md", "lib/README.md"]);
        let app_entry = &index.entries[0];
        assert_eq!(app_entry.rel(), "README.md");
        assert_eq!(index.checkout(app_entry), "app");
        let lib_entry = &index.entries[1];
        assert_eq!(lib_entry.rel(), "README.md");
        assert_eq!(index.checkout(lib_entry), "lib");
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn same_display_in_two_roots_keeps_both() {
        let cwd = unique_dir("ws-file-index-same-display");
        init_repo(&cwd.join("web"));
        init_repo(&cwd.join("legacy/web"));

        let index = build_file_index(
            &cwd,
            vec![root("web", "web/"), root("legacy/web", "web/")],
            MAX_INDEX_ENTRIES,
        );
        let listed: Vec<(&str, &str)> = index
            .entries
            .iter()
            .map(|entry| (entry.display.as_str(), index.checkout(entry)))
            .collect();
        assert_eq!(
            listed,
            vec![("web/README.md", "web"), ("web/README.md", "legacy/web")],
            "dedup is per root; ties keep root order"
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn unmerged_path_lists_once_per_root() {
        let (cwd, app) = workspace_with_app("unmerged");
        git(&app, &["checkout", "-q", "-b", "topic"]);
        write(&app, "README.md", "# topic\n");
        git(&app, &["commit", "-q", "-am", "topic"]);
        git(&app, &["checkout", "-q", "main"]);
        write(&app, "README.md", "# main\n");
        git(&app, &["commit", "-q", "-am", "main"]);
        assert_eq!(
            git::merge_into_head("topic", &app),
            git::MergeIntoHeadResult::Conflict
        );
        let raw = git::list_checkout_files(&app).expect("ls-files");
        assert!(
            raw.iter().filter(|path| *path == "README.md").count() > 1,
            "one line per index stage: {raw:?}"
        );

        let index = build_file_index(&cwd, vec![root("app", "app/")], MAX_INDEX_ENTRIES);
        assert_eq!(displays(&index), vec!["app/README.md"]);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn cap_sets_truncated() {
        let (cwd, app) = workspace_with_app("cap");
        for name in ["b.txt", "c.txt", "d.txt", "e.txt"] {
            write(&app, name, "x\n");
        }

        let full = build_file_index(&cwd, vec![root("app", "")], MAX_INDEX_ENTRIES);
        assert_eq!(full.entries.len(), 5);
        assert!(!full.truncated);

        let capped = build_file_index(&cwd, vec![root("app", "")], 2);
        assert_eq!(capped.entries.len(), 2);
        assert!(capped.truncated);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn non_git_root_lists_nothing_and_records_error() {
        let cwd = unique_dir("ws-file-index-plain");
        write(&cwd, "plain/notes.txt", "not a repo\n");

        let index = build_file_index(&cwd, vec![root("plain", "")], MAX_INDEX_ENTRIES);
        assert!(index.entries.is_empty());
        assert_eq!(index.errors.len(), 1, "{:?}", index.errors);
        assert!(index.errors[0].starts_with("plain: "), "{:?}", index.errors);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn blank_query_returns_first_entries_in_order() {
        let index = index_of(&["c.rs", "a.rs", "b.rs"]);
        let hits = score_files(&index, "  ", 2);
        assert_eq!(
            hits,
            vec![
                FileHit {
                    entry: 0,
                    score: 0,
                    indices: Vec::new()
                },
                FileHit {
                    entry: 1,
                    score: 0,
                    indices: Vec::new()
                },
            ]
        );
        assert_eq!(index.entries[hits[0].entry].display, "a.rs");
    }

    #[test]
    fn subsequence_matches_and_non_matches_drop() {
        let index = index_of(&["src/main.rs", "docs/guide.md"]);
        assert_eq!(hit_displays(&index, "smr"), vec!["src/main.rs"]);
        assert!(score_files(&index, "xyz", MAX_RESULTS).is_empty());
    }

    #[test]
    fn consecutive_run_beats_scattered() {
        assert_outranks(
            &["src/tui/stash_table_extra.rs", "src/tui/state/mod.rs"],
            "state",
            "src/tui/state/mod.rs",
            "src/tui/stash_table_extra.rs",
        );
    }

    #[test]
    fn camel_case_boundary_beats_mid_word() {
        assert_outranks(
            &["src/foobar.ts", "src/fooBar.ts"],
            "fb",
            "src/fooBar.ts",
            "src/foobar.ts",
        );
    }

    #[test]
    fn path_separator_boundary_beats_mid_word() {
        assert_outranks(
            &["src/atmosphere.rs", "src/tui/mod.rs"],
            "tm",
            "src/tui/mod.rs",
            "src/atmosphere.rs",
        );
    }

    #[test]
    fn smart_case_upper_query_is_case_sensitive() {
        let index = index_of(&["README.md", "readme.txt"]);
        assert_eq!(hit_displays(&index, "README"), vec!["README.md"]);
        let mut both = hit_displays(&index, "readme");
        both.sort_unstable();
        assert_eq!(both, vec!["README.md", "readme.txt"]);
    }

    #[test]
    fn indices_mark_matched_chars() {
        let index = index_of(&["src/main.rs"]);
        let hits = score_files(&index, "main", MAX_RESULTS);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].indices, vec![4, 5, 6, 7]);
        assert!(hits[0].score > 0);
    }

    #[test]
    fn indices_are_char_positions_past_combining_marks() {
        let display = "cafe\u{301}/main.rs";
        let index = index_of(&[display]);
        let hits = score_files(&index, "main", MAX_RESULTS);
        assert_eq!(hits.len(), 1);
        let want: Vec<u32> = display
            .chars()
            .enumerate()
            .skip_while(|(_, c)| *c != 'm')
            .take(4)
            .map(|(i, _)| i as u32)
            .collect();
        assert_eq!(want, vec![6, 7, 8, 9]);
        assert_eq!(hits[0].indices, want);
    }

    #[test]
    fn limit_caps_results() {
        let index = index_of(&["a/mod.rs", "b/mod.rs", "c/mod.rs", "d/mod.rs"]);
        assert_eq!(score_files(&index, "mod", 2).len(), 2);
        assert_eq!(score_files(&index, "mod", MAX_RESULTS).len(), 4);
    }

    #[test]
    fn read_text_file_text_binary_and_too_large() {
        let dir = unique_dir("ws-file-index-read");
        fs::create_dir_all(&dir).expect("mkdir");

        let text = dir.join("notes.txt");
        fs::write(&text, "a\tb\r\nxy\x07z\n").expect("write text");
        assert_eq!(
            read_text_file(&text, MAX_FILE_BYTES),
            FileRead::Text {
                lines: vec!["a   b".to_string(), "xy\u{FFFD}z".to_string()],
                max_cols: 5,
            }
        );

        let binary = dir.join("blob.bin");
        fs::write(&binary, b"ab\0cd").expect("write binary");
        assert_eq!(read_text_file(&binary, MAX_FILE_BYTES), FileRead::Binary);

        let len = fs::metadata(&text).expect("meta").len();
        assert_eq!(read_text_file(&text, 4), FileRead::TooLarge { bytes: len });

        assert!(matches!(
            read_text_file(&dir.join("missing.txt"), MAX_FILE_BYTES),
            FileRead::Failed(_)
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn read_text_file_refuses_symlinks_to_non_regular_files() {
        let dir = unique_dir("ws-file-index-special");
        fs::create_dir_all(dir.join("sub")).expect("mkdir");
        let to_dir = dir.join("to_dir");
        std::os::unix::fs::symlink(dir.join("sub"), &to_dir).expect("symlink dir");
        let to_null = dir.join("to_null");
        std::os::unix::fs::symlink("/dev/null", &to_null).expect("symlink null");
        let to_zero = dir.join("to_zero");
        std::os::unix::fs::symlink("/dev/zero", &to_zero).expect("symlink zero");

        let refused = FileRead::Failed("not a regular file".into());
        assert_eq!(read_text_file(&to_dir, MAX_FILE_BYTES), refused);
        assert_eq!(read_text_file(&to_null, MAX_FILE_BYTES), refused);
        assert_eq!(read_text_file(&to_zero, MAX_FILE_BYTES), refused);
        assert_eq!(read_text_file(&dir.join("sub"), MAX_FILE_BYTES), refused);
        let _ = fs::remove_dir_all(&dir);
    }
}

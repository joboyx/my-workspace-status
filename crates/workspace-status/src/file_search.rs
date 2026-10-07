//! Search in files: line hits for a text or regex query over a file index.
//!
//! The files searched are a [`FileIndex`], the same list Quick Open shows
//! (`git ls-files` per checkout in scope, see [`crate::file_index`]). The
//! engine is ripgrep's, in process: `grep-regex` builds the matcher and
//! `grep-searcher` walks the lines. No ignore engine is involved; the index
//! already applied git's ignore rules.
//!
//! Skip rules match the file tab ([`crate::file_index::read_text_file`]):
//! a file over [`MAX_FILE_BYTES`], or with a NUL in its first 8000 bytes,
//! is skipped and counted in [`SearchChunk::skipped`]. A file that is gone
//! or unreadable since the index was built, or is not a regular file, is
//! skipped silently.
//!
//! Caps: one search returns at most [`MAX_SEARCH_HITS`] hits, and a hit
//! line keeps at most [`MAX_HIT_LINE_BYTES`] bytes of its text.
//!
//! Streaming is chunked continuation: [`search_chunk`] searches whole files
//! from a start entry until its time budget is spent, the hit room is
//! full, the file list ends, or the caller cancels. It returns where the
//! next chunk starts. The caller schedules the next chunk while its search
//! is still current.
//!
//! Every function here except [`build_matcher`] reads files. Call them off
//! the TUI draw/event thread.

use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{sinks, BinaryDetection, Searcher, SearcherBuilder};

use crate::file_index::{push_paintable_char, FileIndex, BINARY_SNIFF_BYTES, MAX_FILE_BYTES};

/// Most hits one search returns before it sets [`SearchChunk::capped`].
pub const MAX_SEARCH_HITS: usize = 5000;

/// Most bytes of paintable text one hit line keeps.
pub const MAX_HIT_LINE_BYTES: usize = 512;

/// How a query matches. `Default` is literal text with smart case.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    /// Match case exactly. Off means smart case: a query with no uppercase
    /// letter ignores case, and one with an uppercase letter matches case.
    pub case_sensitive: bool,
    /// Only match where the query is not part of a longer word.
    pub whole_word: bool,
    /// Treat the query as a regular expression. Off means literal text.
    pub regex: bool,
}

/// A compiled query, ready for [`search_chunk`].
#[derive(Clone, Debug)]
pub struct SearchMatcher {
    /// Line-oriented matcher (`\n` is the line terminator).
    inner: RegexMatcher,
}

/// One line that matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// Index into [`FileIndex::entries`].
    pub entry: usize,
    /// 1-based line number in the file.
    pub line: u32,
    /// Paintable line text: lossy UTF-8, no line terminator, tabs expanded
    /// and control characters replaced as the file tab does, at most
    /// [`MAX_HIT_LINE_BYTES`] bytes (cut on a char boundary).
    pub text: String,
    /// Byte ranges `(start, end)` in `text` of every match on the line, in
    /// order. A match past the cut is dropped, one across it is clipped,
    /// and an empty match has no range.
    pub ranges: Vec<(u32, u32)>,
}

/// What one [`search_chunk`] call found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchChunk {
    /// Hits in index order, then line order.
    pub hits: Vec<SearchHit>,
    /// Entry the next chunk starts at. `None` when the file list is done or
    /// the search is capped.
    pub next_entry: Option<usize>,
    /// Files skipped as binary or larger than [`MAX_FILE_BYTES`].
    pub skipped: usize,
    /// True when more hits exist than the room allowed. The search is over.
    pub capped: bool,
}

/// Compile `query` for `options`.
///
/// Cheap enough for the event thread, which uses it to validate the query
/// as it is typed. The error is one line, for an `invalid regex: <error>`
/// status. Literal mode never fails for normal text. An empty query
/// matches every line.
pub fn build_matcher(query: &str, options: SearchOptions) -> Result<SearchMatcher, String> {
    RegexMatcherBuilder::new()
        .fixed_strings(!options.regex)
        .case_smart(!options.case_sensitive)
        .case_insensitive(false)
        .word(options.whole_word)
        .line_terminator(Some(b'\n'))
        .build(query)
        .map(|inner| SearchMatcher { inner })
        .map_err(|err| one_line_error(&err.to_string()))
}

/// The `error: ...` line of a multi-line regex syntax error, else every
/// line joined with spaces.
fn one_line_error(message: &str) -> String {
    let lines = message.lines().map(str::trim).filter(|l| !l.is_empty());
    if let Some(reason) = lines.clone().find_map(|l| l.strip_prefix("error: ")) {
        return reason.to_string();
    }
    lines.collect::<Vec<_>>().join(" ")
}

/// Search `index` entries from `start` with `matcher`, one chunk.
///
/// Whole files are searched in index order. The chunk ends after the
/// first file whose end finds `budget` spent (at least one file is always
/// searched, so a zero budget still advances), at the end of the list, or
/// before the next file once `cancelled` returns true. `room` is how many
/// more hits the search may return; a hit past it ends the search with
/// `capped` set and exactly `room` hits. Paths resolve as `cwd` /
/// checkout / relative path.
pub fn search_chunk(
    cwd: &Path,
    index: &FileIndex,
    matcher: &SearchMatcher,
    start: usize,
    room: usize,
    budget: Duration,
    cancelled: impl Fn() -> bool,
) -> SearchChunk {
    let started = Instant::now();
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::none())
        .line_number(true)
        .build();
    let mut chunk = SearchChunk::default();
    let mut entry = start;
    while entry < index.entries.len() {
        if cancelled() {
            chunk.next_entry = Some(entry);
            return chunk;
        }
        let file = &index.entries[entry];
        let path = cwd.join(index.checkout(file)).join(file.rel());
        match read_searchable(&path) {
            Searchable::Text(bytes) => {
                let room_left = room - chunk.hits.len();
                let full = search_file(
                    &mut searcher,
                    matcher,
                    entry,
                    &bytes,
                    room_left,
                    &mut chunk.hits,
                );
                if full {
                    chunk.capped = true;
                    return chunk;
                }
            }
            Searchable::Skipped => chunk.skipped += 1,
            Searchable::Gone => {}
        }
        entry += 1;
        if entry < index.entries.len() && started.elapsed() >= budget {
            chunk.next_entry = Some(entry);
            return chunk;
        }
    }
    chunk
}

/// A file's bytes, or why it is not searched.
enum Searchable {
    /// Text to search.
    Text(Vec<u8>),
    /// Binary or too large: counted in [`SearchChunk::skipped`].
    Skipped,
    /// Missing, unreadable, or not a regular file: skipped silently.
    Gone,
}

/// Read `path` for search, with the file tab's size and binary rules.
fn read_searchable(path: &Path) -> Searchable {
    // `metadata` follows symlinks; check the target before `open`, which
    // blocks on a FIFO and never ends on a device such as `/dev/zero`.
    let meta = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta,
        _ => return Searchable::Gone,
    };
    if meta.len() > MAX_FILE_BYTES {
        return Searchable::Skipped;
    }
    let mut raw = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut raw));
    if read.is_err() {
        return Searchable::Gone;
    }
    // Over the cap means the file grew after the metadata call.
    if raw.len() as u64 > MAX_FILE_BYTES || raw[..raw.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return Searchable::Skipped;
    }
    Searchable::Text(raw)
}

/// Push the hits of one file into `hits`, at most `room` of them.
///
/// Returns true when a hit past `room` was found (the search is capped).
fn search_file(
    searcher: &mut Searcher,
    matcher: &SearchMatcher,
    entry: usize,
    bytes: &[u8],
    room: usize,
    hits: &mut Vec<SearchHit>,
) -> bool {
    let mut found = 0usize;
    let mut full = false;
    let sink = sinks::Bytes(|line_number, line| {
        if found == room {
            full = true;
            return Ok(false);
        }
        let mut matches = Vec::new();
        // A `RegexMatcher` never fails a search (`NoError`).
        let _ = matcher.inner.find_iter(line, |m| {
            matches.push((m.start(), m.end()));
            true
        });
        let (text, ranges) = hit_text(line, &matches);
        hits.push(SearchHit {
            entry,
            line: u32::try_from(line_number).unwrap_or(u32::MAX),
            text,
            ranges,
        });
        found += 1;
        Ok(true)
    });
    // Searching an in-memory slice with a sink that never fails has no
    // error to report.
    let _ = searcher.search_slice(&matcher.inner, bytes, sink);
    full
}

/// Paintable text of a matched line plus its match ranges remapped into it.
///
/// `line` is the raw line as the searcher reports it, terminator included;
/// `matches` are byte ranges into it. Each source unit (a char, or one
/// invalid UTF-8 sequence that lossy decoding turns into U+FFFD) goes
/// through the file tab's per-char sanitizer. Units stop before the one
/// that would push the text past [`MAX_HIT_LINE_BYTES`].
fn hit_text(line: &[u8], matches: &[(usize, usize)]) -> (String, Vec<(u32, u32)>) {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut text = String::new();
    let mut col = 0usize;
    // Per source byte kept: where its unit starts and ends in `text`.
    let mut unit_start: Vec<usize> = Vec::new();
    let mut unit_end: Vec<usize> = Vec::new();
    let mut push_unit = |ch: char, source_len: usize| -> bool {
        let before = text.len();
        push_paintable_char(&mut text, &mut col, ch);
        if text.len() > MAX_HIT_LINE_BYTES {
            text.truncate(before);
            return false;
        }
        unit_start.extend(std::iter::repeat_n(before, source_len));
        unit_end.extend(std::iter::repeat_n(text.len(), source_len));
        true
    };
    'units: for part in line.utf8_chunks() {
        for ch in part.valid().chars() {
            if !push_unit(ch, ch.len_utf8()) {
                break 'units;
            }
        }
        if !part.invalid().is_empty() && !push_unit('\u{FFFD}', part.invalid().len()) {
            break;
        }
    }
    let kept = unit_start.len();
    let ranges = matches
        .iter()
        .filter(|&&(start, end)| start < kept && end > start)
        .map(|&(start, end)| {
            let end = if end > kept {
                text.len()
            } else {
                unit_end[end - 1]
            };
            (unit_start[start] as u32, end as u32)
        })
        .collect();
    (text, ranges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::{build_file_index, IndexRoot, MAX_INDEX_ENTRIES};
    use crate::testutil::{init_repo, unique_dir};
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;

    /// Workspace dir with one repo at `<cwd>/app`, plus `files` written
    /// (untracked files are listed too).
    fn workspace(tag: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let cwd = unique_dir(&format!("ws-file-search-{tag}"));
        let app = cwd.join("app");
        init_repo(&app);
        for (rel, body) in files {
            let path = app.join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, body).expect("write fixture");
        }
        cwd
    }

    fn index_of(cwd: &Path) -> FileIndex {
        let roots = vec![IndexRoot {
            checkout: "app".to_string(),
            prefix: String::new(),
        }];
        let index = build_file_index(cwd, roots, MAX_INDEX_ENTRIES);
        assert!(index.errors.is_empty(), "{:?}", index.errors);
        index
    }

    fn matcher(query: &str, options: SearchOptions) -> SearchMatcher {
        build_matcher(query, options).expect("query compiles")
    }

    /// One chunk with room and time for everything.
    fn search_all(
        cwd: &Path,
        index: &FileIndex,
        query: &str,
        options: SearchOptions,
    ) -> SearchChunk {
        let chunk = search_chunk(
            cwd,
            index,
            &matcher(query, options),
            0,
            MAX_SEARCH_HITS,
            Duration::from_secs(60),
            || false,
        );
        assert_eq!(chunk.next_entry, None);
        chunk
    }

    /// `(display, line)` of every hit.
    fn found<'a>(index: &'a FileIndex, chunk: &SearchChunk) -> Vec<(&'a str, u32)> {
        chunk
            .hits
            .iter()
            .map(|h| (index.entries[h.entry].display.as_str(), h.line))
            .collect()
    }

    fn regex() -> SearchOptions {
        SearchOptions {
            regex: true,
            ..SearchOptions::default()
        }
    }

    #[test]
    fn literal_text_vs_regex() {
        let cwd = workspace("literal", &[("a.txt", b"abc\na.c\n")]);
        let index = index_of(&cwd);

        let literal = search_all(&cwd, &index, "a.c", SearchOptions::default());
        assert_eq!(found(&index, &literal), vec![("a.txt", 2)]);
        assert_eq!(literal.hits[0].text, "a.c");

        let pattern = search_all(&cwd, &index, "a.c", regex());
        assert_eq!(found(&index, &pattern), vec![("a.txt", 1), ("a.txt", 2)]);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn smart_case_and_case_chip() {
        let cwd = workspace("case", &[("a.txt", b"Widget\nwidget\n")]);
        let index = index_of(&cwd);

        let lower = search_all(&cwd, &index, "widget", SearchOptions::default());
        assert_eq!(found(&index, &lower), vec![("a.txt", 1), ("a.txt", 2)]);

        let upper = search_all(&cwd, &index, "Widget", SearchOptions::default());
        assert_eq!(found(&index, &upper), vec![("a.txt", 1)]);

        let exact = SearchOptions {
            case_sensitive: true,
            ..SearchOptions::default()
        };
        let forced = search_all(&cwd, &index, "widget", exact);
        assert_eq!(found(&index, &forced), vec![("a.txt", 2)]);

        let regex_lower = search_all(&cwd, &index, "w.dget", regex());
        assert_eq!(regex_lower.hits.len(), 2, "smart case in regex mode too");
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn whole_word() {
        let cwd = workspace("word", &[("a.txt", b"cat\ncatalog\nthe cat sat\n")]);
        let index = index_of(&cwd);
        let options = SearchOptions {
            whole_word: true,
            ..SearchOptions::default()
        };

        let chunk = search_all(&cwd, &index, "cat", options);
        assert_eq!(found(&index, &chunk), vec![("a.txt", 1), ("a.txt", 3)]);
        assert_eq!(chunk.hits[0].ranges, vec![(0, 3)]);
        assert_eq!(chunk.hits[1].ranges, vec![(4, 7)]);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn every_match_on_a_line_and_one_based_lines() {
        let cwd = workspace(
            "ranges",
            &[
                ("a.txt", b"none\nnone\nab x ab ab\nab\n"),
                ("b.txt", b"ab\n"),
            ],
        );
        let index = index_of(&cwd);

        let chunk = search_all(&cwd, &index, "ab", SearchOptions::default());
        assert_eq!(
            found(&index, &chunk),
            vec![("a.txt", 3), ("a.txt", 4), ("b.txt", 1)]
        );
        assert_eq!(chunk.hits[0].text, "ab x ab ab");
        assert_eq!(chunk.hits[0].ranges, vec![(0, 2), (5, 7), (8, 10)]);
        assert_eq!(chunk.hits[1].ranges, vec![(0, 2)]);
        assert_eq!(chunk.skipped, 0);
        assert!(!chunk.capped);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn text_is_paintable_and_ranges_follow_it() {
        let cwd = workspace(
            "paint",
            &[
                ("tabs.txt", b"\tfoo\tbar\r\n"),
                ("ctrl.txt", b"\x07bar\n"),
                ("bad.txt", b"\xffbar\n"),
            ],
        );
        let index = index_of(&cwd);

        let chunk = search_all(&cwd, &index, "bar", SearchOptions::default());
        let by_file = |name: &str| {
            chunk
                .hits
                .iter()
                .find(|h| index.entries[h.entry].display == name)
                .unwrap_or_else(|| panic!("no hit in {name}"))
        };
        // `\t` at col 0 pads to 4, `\t` after "foo" (col 7) pads to 8.
        let tabs = by_file("tabs.txt");
        assert_eq!(tabs.text, "    foo bar");
        assert_eq!(tabs.ranges, vec![(8, 11)]);
        // The control char and the invalid byte each become U+FFFD (3 bytes).
        for name in ["ctrl.txt", "bad.txt"] {
            let hit = by_file(name);
            assert_eq!(hit.text, "\u{FFFD}bar", "{name}");
            assert_eq!(hit.ranges, vec![(3, 6)], "{name}");
        }

        let across_tab = search_all(&cwd, &index, "foo\tb", SearchOptions::default());
        assert_eq!(across_tab.hits[0].ranges, vec![(4, 9)]);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn long_line_is_cut_on_a_char_boundary_and_ranges_clip() {
        let past = format!("{}needle\n", "x".repeat(600));
        let across = format!("{}needle\n", "y".repeat(510));
        let wide = format!("{}\u{e9}needle\n", "z".repeat(511));
        let cwd = workspace(
            "cap-line",
            &[
                ("a_past.txt", past.as_bytes()),
                ("b_across.txt", across.as_bytes()),
                ("c_wide.txt", wide.as_bytes()),
            ],
        );
        let index = index_of(&cwd);

        let chunk = search_all(&cwd, &index, "needle", SearchOptions::default());
        assert_eq!(chunk.hits.len(), 3);
        let [past, across, wide] = &chunk.hits[..] else {
            unreachable!()
        };
        assert_eq!(past.text, "x".repeat(MAX_HIT_LINE_BYTES));
        assert!(past.ranges.is_empty(), "{:?}", past.ranges);
        assert_eq!(across.text.len(), MAX_HIT_LINE_BYTES);
        assert!(across.text.ends_with("yne"), "{:?}", across.text);
        assert_eq!(across.ranges, vec![(510, 512)]);
        // "é" is 2 bytes and would end at 513: the cut lands before it.
        assert_eq!(wide.text, "z".repeat(511));
        assert!(wide.ranges.is_empty());
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn binary_and_too_large_files_are_counted_not_searched() {
        let mut big = vec![b'a'; MAX_FILE_BYTES as usize];
        big.extend_from_slice(b"\nneedle\n");
        let cwd = workspace(
            "skip",
            &[
                ("big.txt", &big),
                ("blob.bin", b"needle\0needle\n"),
                ("ok.txt", b"needle\n"),
            ],
        );
        let index = index_of(&cwd);

        let chunk = search_all(&cwd, &index, "needle", SearchOptions::default());
        assert_eq!(found(&index, &chunk), vec![("ok.txt", 1)]);
        assert_eq!(chunk.skipped, 2);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn file_deleted_after_the_index_is_skipped_silently() {
        let cwd = workspace(
            "gone",
            &[("gone.txt", b"needle\n"), ("ok.txt", b"needle\n")],
        );
        let index = index_of(&cwd);
        fs::remove_file(cwd.join("app/gone.txt")).expect("rm");

        let chunk = search_all(&cwd, &index, "needle", SearchOptions::default());
        assert_eq!(found(&index, &chunk), vec![("ok.txt", 1)]);
        assert_eq!(chunk.skipped, 0);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_file_is_skipped_silently() {
        let cwd = workspace("special", &[("sub/a.txt", b"needle\n")]);
        std::os::unix::fs::symlink("sub", cwd.join("app/to_dir")).expect("symlink dir");
        let index = index_of(&cwd);
        assert!(index.entries.iter().any(|e| e.display == "to_dir"));

        let chunk = search_all(&cwd, &index, "needle", SearchOptions::default());
        assert_eq!(found(&index, &chunk), vec![("sub/a.txt", 1)]);
        assert_eq!(chunk.skipped, 0);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn hit_room_caps_the_search() {
        let cwd = workspace(
            "cap",
            &[("a.txt", b"hit\nhit\nhit\n"), ("b.txt", b"hit\nhit\n")],
        );
        let index = index_of(&cwd);
        let query = matcher("hit", SearchOptions::default());
        let run = |room| {
            search_chunk(
                &cwd,
                &index,
                &query,
                0,
                room,
                Duration::from_secs(60),
                || false,
            )
        };

        for room in [2, 3, 4] {
            let chunk = run(room);
            assert_eq!(chunk.hits.len(), room, "room {room}");
            assert!(chunk.capped, "room {room}");
            assert_eq!(chunk.next_entry, None, "room {room}");
        }
        let exact = run(5);
        assert_eq!(exact.hits.len(), 5);
        assert!(!exact.capped, "no hit past the room");
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn chunks_continue_until_every_hit_is_found_once() {
        let cwd = workspace(
            "chunks",
            &[
                ("a.txt", b"hit\nmiss\nhit\n"),
                ("b.txt", b"miss\n"),
                ("c.txt", b"hit\n"),
                ("d.bin", b"hit\0\n"),
            ],
        );
        let index = index_of(&cwd);
        let query = matcher("hit", SearchOptions::default());
        let whole = search_all(&cwd, &index, "hit", SearchOptions::default());

        let mut hits = Vec::new();
        let mut skipped = 0;
        let mut starts = Vec::new();
        let mut next = Some(0);
        while let Some(start) = next {
            starts.push(start);
            let chunk = search_chunk(
                &cwd,
                &index,
                &query,
                start,
                MAX_SEARCH_HITS - hits.len(),
                Duration::ZERO,
                || false,
            );
            assert!(!chunk.capped);
            hits.extend(chunk.hits);
            skipped += chunk.skipped;
            next = chunk.next_entry;
        }
        // README.md, a.txt, b.txt, c.txt, d.bin: one file per zero-budget chunk.
        assert_eq!(index.entries.len(), 5);
        assert_eq!(starts, vec![0, 1, 2, 3, 4]);
        assert_eq!(hits, whole.hits);
        assert_eq!(skipped, whole.skipped);
        assert_eq!(skipped, 1);
        assert_eq!(
            found(&index, &whole),
            vec![("a.txt", 1), ("a.txt", 3), ("c.txt", 1)]
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn cancel_stops_before_the_next_file() {
        let cwd = workspace("cancel", &[("a.txt", b"hit\n"), ("b.txt", b"hit\n")]);
        let index = index_of(&cwd);
        let query = matcher("hit", SearchOptions::default());
        let long = Duration::from_secs(60);

        let at_once = search_chunk(&cwd, &index, &query, 1, MAX_SEARCH_HITS, long, || true);
        assert!(at_once.hits.is_empty());
        assert_eq!(at_once.next_entry, Some(1));

        // Cancelled after the first file (a.txt at entry 1) was searched.
        let calls = Cell::new(0);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() > 1
        };
        let after_one = search_chunk(&cwd, &index, &query, 1, MAX_SEARCH_HITS, long, cancelled);
        assert_eq!(found(&index, &after_one), vec![("a.txt", 1)]);
        assert_eq!(after_one.next_entry, Some(2));
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn invalid_regex_is_an_error_only_in_regex_mode() {
        let err = build_matcher("a(b", regex()).expect_err("unclosed group");
        assert!(!err.is_empty());
        assert!(!err.contains('\n'), "{err:?}");
        assert_eq!(err, "unclosed group");
        assert!(build_matcher("a(b", SearchOptions::default()).is_ok());
        assert!(build_matcher(
            "a(b",
            SearchOptions {
                whole_word: true,
                ..SearchOptions::default()
            }
        )
        .is_ok());
    }
}

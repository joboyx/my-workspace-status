//! Explorer model: a lazy, VS Code-style file tree for one git checkout.
//!
//! Each folder is listed only when it is expanded ([`list_dir`], blocking,
//! run on the worker pool). Ignored entries (`node_modules/`, `target/`, …)
//! are always listed and carry [`ExplorerEntry::ignored`] so the paint can
//! dim them. Git status letters and folder dirty dots come from the
//! in-memory workspace snapshot through [`ExplorerStatus`]; the tree itself
//! ([`ExplorerTree`]) is pure and does no I/O.
//!
//! Paths are `/`-separated and relative to the checkout. The checkout root
//! is `""`.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use crate::git::{git_binary, git_process};
use crate::snapshot::FileChange;

use super::icons::{status_letter_from_change, FileStatusLetter};

/// One child of a listed folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerEntry {
    /// File or folder name, no `/`. A name that is not valid UTF-8 is
    /// converted lossily (U+FFFD), so it does not round-trip to a disk
    /// path.
    pub name: String,
    /// True for a real directory. A symlink to a directory is a file here,
    /// so the tree never follows a link loop.
    pub is_dir: bool,
    /// Git ignores this entry (or one of its ancestor folders).
    pub ignored: bool,
}

/// List the children of `rel_dir` inside `checkout`. BLOCKING: run it on
/// the worker pool, never on the TUI event thread.
///
/// `rel_dir` is `""` for the checkout root, else a `/`-separated path
/// relative to `checkout`. The `.git` entry (directory or gitfile) is
/// skipped. Entries come back folders first, then files, each group
/// ordered by name ignoring case (ties by exact name).
///
/// When `parent_ignored` is true every child is ignored and git is not
/// asked ([`ExplorerTree::is_ignored_dir`] says what to pass). Otherwise one
/// `git check-ignore` call over the children decides the flag.
///
/// Names that are not valid UTF-8 are converted lossily (see
/// [`ExplorerEntry::name`]).
pub fn list_dir(
    checkout: &Path,
    rel_dir: &str,
    parent_ignored: bool,
) -> std::io::Result<Vec<ExplorerEntry>> {
    let dir = if rel_dir.is_empty() {
        checkout.to_path_buf()
    } else {
        checkout.join(rel_dir)
    };
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        // `DirEntry::file_type` does not follow symlinks.
        let is_dir = entry.file_type()?.is_dir();
        entries.push(ExplorerEntry {
            name,
            is_dir,
            ignored: parent_ignored,
        });
    }
    if !parent_ignored && !entries.is_empty() {
        let rels: Vec<String> = entries
            .iter()
            .map(|entry| join_rel(rel_dir, &entry.name))
            .collect();
        // A git failure (no git, a nested repository, a broken index) must
        // not hide the folder: the entries stay un-ignored and still list.
        if let Some(ignored) = check_ignore(checkout, &rels) {
            for (entry, rel) in entries.iter_mut().zip(&rels) {
                entry.ignored = ignored.contains(rel);
            }
        }
    }
    entries.sort_by(|a, b| entry_order(a.is_dir, &a.name, b.is_dir, &b.name));
    Ok(entries)
}

/// The subset of `rels` that git ignores, from one `git check-ignore -z
/// --stdin` run in `checkout`. `None` when git fails; exit code 1 means
/// "none ignored" and is an empty set.
fn check_ignore(checkout: &Path, rels: &[String]) -> Option<HashSet<String>> {
    let mut cmd = git_process(git_binary());
    cmd.args(["check-ignore", "-z", "--stdin"])
        .current_dir(checkout)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        // check-ignore rejects these global pathspec modes with a fatal
        // error, so a value inherited from the user's shell must not reach it.
        .env_remove("GIT_LITERAL_PATHSPECS")
        .env_remove("GIT_GLOB_PATHSPECS")
        .env_remove("GIT_ICASE_PATHSPECS");
    let mut child = cmd.spawn().ok()?;
    let mut input = Vec::new();
    for rel in rels {
        // git always parses a leading `:` as pathspec magic, and
        // check-ignore fails the whole run on magic it does not support
        // (`:!x`, `:(glob)x`). A `./` prefix keeps a root-level name
        // literal; git echoes the prefix back and the parse below strips it.
        if rel.starts_with(':') {
            input.extend_from_slice(b"./");
        }
        input.extend_from_slice(rel.as_bytes());
        input.push(0);
    }
    let mut stdin = child.stdin.take()?;
    // Write on a second thread: git answers as it reads, so a large folder
    // could fill the stdout pipe while we still block on stdin.
    let out = std::thread::scope(|scope| {
        scope.spawn(move || {
            // A write error means git exited early; its status says why.
            let _ = stdin.write_all(&input);
        });
        child.wait_with_output()
    })
    .ok()?;
    match out.status.code() {
        Some(0) => Some(
            out.stdout
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
                .map(|path| {
                    let path = String::from_utf8_lossy(path);
                    path.strip_prefix("./").unwrap_or(&path).to_string()
                })
                .collect(),
        ),
        Some(1) => Some(HashSet::new()),
        _ => None,
    }
}

/// Explorer sort: folders first, then name ignoring case, then exact name.
fn entry_order(a_dir: bool, a_name: &str, b_dir: bool, b_name: &str) -> Ordering {
    b_dir
        .cmp(&a_dir)
        .then_with(|| {
            a_name
                .chars()
                .flat_map(char::to_lowercase)
                .cmp(b_name.chars().flat_map(char::to_lowercase))
        })
        .then_with(|| a_name.cmp(b_name))
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Parent folder of `rel` (`""` for a root-level entry).
fn parent_rel(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// True when `rel` is strictly inside folder `dir` (`""` holds everything).
fn is_under(rel: &str, dir: &str) -> bool {
    if dir.is_empty() {
        !rel.is_empty()
    } else {
        rel.len() > dir.len() && rel.starts_with(dir) && rel.as_bytes()[dir.len()] == b'/'
    }
}

/// Every proper ancestor folder of `rel`, root (`""`) first.
fn ancestors(rel: &str) -> Vec<&str> {
    let mut out = vec![""];
    for (idx, byte) in rel.bytes().enumerate() {
        if byte == b'/' {
            out.push(&rel[..idx]);
        }
    }
    out
}

/// Git status of one checkout, indexed for paint-time lookups.
///
/// Built once per snapshot from the checkout's [`FileChange`] list; every
/// lookup is a hash probe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExplorerStatus {
    letters: HashMap<String, FileStatusLetter>,
    dirty_dirs: HashSet<String>,
    deleted: HashMap<String, Vec<String>>,
    untracked: HashSet<String>,
}

impl ExplorerStatus {
    /// Index `changes` (one checkout's snapshot changes).
    ///
    /// Letters use [`status_letter_from_change`], the Workspace tree's
    /// mapping, so an untracked file is `A` there and here.
    pub fn from_changes(changes: &[FileChange]) -> Self {
        let mut status = Self::default();
        for change in changes {
            let path = change.path.trim_end_matches('/');
            if path.is_empty() {
                continue;
            }
            let letter = status_letter_from_change(change);
            for dir in ancestors(path) {
                status.dirty_dirs.insert(dir.to_string());
            }
            if change.untracked {
                status.untracked.insert(path.to_string());
            }
            // Either side: `MD` / `AD` (staged, then removed from disk) have
            // a letter other than `D` but are still gone from disk.
            let deleted = change.unstaged_status.as_deref() == Some("D")
                || change.staged_status.as_deref() == Some("D");
            if deleted {
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                status
                    .deleted
                    .entry(parent_rel(path).to_string())
                    .or_default()
                    .push(name);
            }
            status.letters.insert(path.to_string(), letter);
        }
        status
    }

    /// Status letter for the file at `rel`, or `None` when it is unchanged.
    ///
    /// The paint takes the badge, colour, and icon kind from the letter.
    pub fn letter(&self, rel: &str) -> Option<FileStatusLetter> {
        self.letters.get(rel).copied()
    }

    /// True when the file at `rel` is untracked. [`ExplorerStatus::letter`]
    /// says `A` for it (the Workspace tree's mapping); the Explorer paints
    /// `??` instead.
    pub fn is_untracked(&self, rel: &str) -> bool {
        self.untracked.contains(rel)
    }

    /// True when any changed path is inside folder `rel_dir` at any depth.
    pub fn dir_dirty(&self, rel_dir: &str) -> bool {
        self.dirty_dirs.contains(rel_dir)
    }

    /// Names of the direct children of `rel_dir` that git reports as
    /// deleted (`D` on the staged or the unstaged side), so the tree can still show a file that is gone from
    /// disk.
    pub fn deleted_in(&self, rel_dir: &str) -> &[String] {
        self.deleted.get(rel_dir).map_or(&[], Vec::as_slice)
    }
}

/// One painted Explorer row, from [`ExplorerTree::rows`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerRow {
    /// Path relative to the checkout. On a placeholder row this is the
    /// folder that is still loading.
    pub rel: String,
    /// File or folder name (empty on a placeholder row).
    pub name: String,
    /// Indent level; root-level entries are 0.
    pub depth: usize,
    /// True for a folder row.
    pub is_dir: bool,
    /// True for a folder row that is expanded.
    pub expanded: bool,
    /// Git ignores this entry; paint it dim.
    pub ignored: bool,
    /// The one "loading" child of an expanded folder whose listing has not
    /// arrived yet. The cursor never lands on it.
    pub placeholder: bool,
}

/// Listing state of one folder.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Listing {
    /// Requested from the pool; not arrived yet.
    Loading,
    /// Children on disk, in [`list_dir`] order, plus their names for O(1)
    /// lookups (a status path against a large folder).
    Loaded {
        entries: Vec<ExplorerEntry>,
        names: HashSet<String>,
    },
}

/// Explorer model for one checkout: folder listings, the expanded set,
/// and the cursor. Pure — the caller runs [`list_dir`] for every folder
/// rel a method returns and hands the result to
/// [`ExplorerTree::apply_listing`].
///
/// A new tree has no root listing: call `expand("")` to get the root load
/// request. On a listing error, apply an empty listing so the folder stops
/// showing its loading row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerTree {
    listings: HashMap<String, Listing>,
    /// Expanded folder rels. The root (`""`) is always in it.
    expanded: HashSet<String>,
    /// Rel path of the focused row; `None` means the first row.
    cursor: Option<String>,
}

impl Default for ExplorerTree {
    fn default() -> Self {
        Self::new()
    }
}

impl ExplorerTree {
    /// Empty tree with only the root expanded and nothing loaded.
    pub fn new() -> Self {
        Self {
            listings: HashMap::new(),
            expanded: HashSet::from([String::new()]),
            cursor: None,
        }
    }

    /// Store the listing of `rel_dir`. The cursor stays on the same rel
    /// path when that path is still a row.
    pub fn apply_listing(&mut self, rel_dir: &str, entries: Vec<ExplorerEntry>) {
        let names = entries.iter().map(|entry| entry.name.clone()).collect();
        self.listings
            .insert(rel_dir.to_string(), Listing::Loaded { entries, names });
    }

    /// True when folder `rel_dir` is expanded (the root always is).
    pub fn is_expanded(&self, rel_dir: &str) -> bool {
        self.expanded.contains(rel_dir)
    }

    /// True when folder `rel_dir` is ignored, per its parent's listing.
    /// Pass this as `parent_ignored` to [`list_dir`] for that folder.
    pub fn is_ignored_dir(&self, rel_dir: &str) -> bool {
        if rel_dir.is_empty() {
            return false;
        }
        let name = rel_dir.rsplit('/').next().unwrap_or(rel_dir);
        match self.listings.get(parent_rel(rel_dir)) {
            Some(Listing::Loaded { entries, .. }) => entries
                .iter()
                .any(|entry| entry.is_dir && entry.ignored && entry.name == name),
            _ => false,
        }
    }

    /// Mark `rel_dir` as requested when it has no listing and no request in
    /// flight. Returns `rel_dir` when the caller must load it.
    fn request(&mut self, rel_dir: &str) -> Option<String> {
        if self.listings.contains_key(rel_dir) {
            return None;
        }
        self.listings.insert(rel_dir.to_string(), Listing::Loading);
        Some(rel_dir.to_string())
    }

    /// Expand folder `rel_dir`. Returns the folder to load when its
    /// listing is missing and not already requested.
    pub fn expand(&mut self, rel_dir: &str) -> Option<String> {
        self.expanded.insert(rel_dir.to_string());
        self.request(rel_dir)
    }

    /// Collapse folder `rel_dir` (the root stays expanded). A cursor inside
    /// it moves to the folder row. The listing stays cached.
    pub fn collapse(&mut self, rel_dir: &str) {
        if rel_dir.is_empty() {
            return;
        }
        self.expanded.remove(rel_dir);
        if self
            .cursor
            .as_deref()
            .is_some_and(|cursor| is_under(cursor, rel_dir))
        {
            self.cursor = Some(rel_dir.to_string());
        }
    }

    /// Collapse an expanded folder, else expand it. Returns the folder to
    /// load, as [`ExplorerTree::expand`] does.
    pub fn toggle(&mut self, rel_dir: &str) -> Option<String> {
        if !rel_dir.is_empty() && self.is_expanded(rel_dir) {
            self.collapse(rel_dir);
            None
        } else {
            self.expand(rel_dir)
        }
    }

    /// Expand every ancestor folder of `rel_path` (and `rel_path` itself
    /// when `is_dir`), then move the cursor to it. Returns the folders that
    /// still need a listing, root first.
    pub fn reveal(&mut self, rel_path: &str, is_dir: bool) -> Vec<String> {
        let mut dirs = ancestors(rel_path);
        if is_dir && !rel_path.is_empty() {
            dirs.push(rel_path);
        }
        let mut to_load = Vec::new();
        for dir in dirs {
            if let Some(load) = self.expand(dir) {
                to_load.push(load);
            }
        }
        if !rel_path.is_empty() {
            self.cursor = Some(rel_path.to_string());
        }
        to_load
    }

    /// Folders on the path to `rel_path` whose cached listing lacks the
    /// next step of that path (a file or folder created after the listing
    /// was taken), root first. The caller lists them again; their old
    /// listing stays painted until the new one lands.
    pub fn stale_on_path(&self, rel_path: &str) -> Vec<String> {
        if rel_path.is_empty() {
            return Vec::new();
        }
        let mut steps = ancestors(rel_path);
        steps.remove(0);
        steps.push(rel_path);
        steps
            .into_iter()
            .filter_map(|step| {
                let parent = parent_rel(step);
                let name = step.rsplit('/').next().unwrap_or(step);
                match self.listings.get(parent) {
                    Some(Listing::Loaded { names, .. }) if !names.contains(name) => {
                        Some(parent.to_string())
                    }
                    _ => None,
                }
            })
            .collect()
    }

    /// True when folder `rel_dir` is expanded and every ancestor folder is
    /// too, so its rows paint.
    pub fn is_open_and_visible(&self, rel_dir: &str) -> bool {
        self.expanded.contains(rel_dir)
            && ancestors(rel_dir)
                .iter()
                .all(|dir| self.expanded.contains(*dir))
    }

    /// Drop the cached listings of collapsed folders (they reload on the
    /// next expand) and return every visible expanded folder with a loaded
    /// listing, root first, for the caller to re-list. A folder under a
    /// collapsed ancestor keeps its listing but is not re-listed. The old
    /// listing stays painted until the new one arrives.
    pub fn invalidate(&mut self) -> Vec<String> {
        let expanded = &self.expanded;
        self.listings
            .retain(|rel, listing| expanded.contains(rel) || *listing == Listing::Loading);
        let mut dirs: Vec<String> = self
            .listings
            .iter()
            .filter(|(rel, listing)| {
                matches!(listing, Listing::Loaded { .. })
                    && expanded.contains(*rel)
                    && ancestors(rel.as_str())
                        .iter()
                        .all(|dir| expanded.contains(*dir))
            })
            .map(|(rel, _)| rel.clone())
            .collect();
        dirs.sort_by(|a, b| {
            let depth = |rel: &str| rel.split('/').filter(|part| !part.is_empty()).count();
            depth(a).cmp(&depth(b)).then_with(|| a.cmp(b))
        });
        dirs
    }

    /// Flatten the expanded folders depth-first into painted rows.
    ///
    /// Files that `status` reports deleted but that are missing from the
    /// disk listing are merged in as (non-ignored) file rows, sorted into
    /// place. Files inside a folder that is deleted as a whole are not
    /// shown: that folder is not on disk, so no listing names it. An
    /// expanded folder whose listing has not arrived gets one placeholder
    /// child row.
    pub fn rows(&self, status: &ExplorerStatus) -> Vec<ExplorerRow> {
        let mut out = Vec::new();
        self.walk("", 0, false, status, &mut out);
        out
    }

    fn walk(
        &self,
        dir: &str,
        depth: usize,
        dir_ignored: bool,
        status: &ExplorerStatus,
        out: &mut Vec<ExplorerRow>,
    ) {
        let Some(Listing::Loaded {
            entries: listed,
            names,
        }) = self.listings.get(dir)
        else {
            out.push(ExplorerRow {
                rel: dir.to_string(),
                name: String::new(),
                depth,
                is_dir: false,
                expanded: false,
                ignored: dir_ignored,
                placeholder: true,
            });
            return;
        };
        let missing: Vec<&String> = status
            .deleted_in(dir)
            .iter()
            .filter(|name| !names.contains(name.as_str()))
            .collect();
        // Borrow the listing; copy it only when deleted files merge in.
        let entries: Cow<'_, [ExplorerEntry]> = if missing.is_empty() {
            Cow::Borrowed(listed)
        } else {
            let mut merged = listed.clone();
            merged.extend(missing.into_iter().map(|name| ExplorerEntry {
                name: name.clone(),
                is_dir: false,
                ignored: false,
            }));
            merged.sort_by(|a, b| entry_order(a.is_dir, &a.name, b.is_dir, &b.name));
            Cow::Owned(merged)
        };
        for entry in entries.iter() {
            let rel = join_rel(dir, &entry.name);
            let expanded = entry.is_dir && self.expanded.contains(&rel);
            out.push(ExplorerRow {
                rel: rel.clone(),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
                ignored: entry.ignored,
                placeholder: false,
            });
            if expanded {
                self.walk(&rel, depth + 1, entry.ignored, status, out);
            }
        }
    }

    /// Rel path of the focused row as stored. `None` before the first move.
    #[cfg(test)]
    pub fn cursor_rel(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Focus the row at `rel`.
    pub fn set_cursor_rel(&mut self, rel: &str) {
        self.cursor = Some(rel.to_string());
    }

    /// Index of the focused row in `rows`. A cursor path that is no longer
    /// a row falls back to its nearest visible ancestor folder, then to the
    /// first non-placeholder row. `None` when no row can take focus.
    pub fn cursor_index(&self, rows: &[ExplorerRow]) -> Option<usize> {
        let find = |rel: &str| {
            rows.iter()
                .position(|row| !row.placeholder && row.rel == rel)
        };
        if let Some(cursor) = self.cursor.as_deref() {
            if let Some(idx) = find(cursor) {
                return Some(idx);
            }
            if let Some(idx) = ancestors(cursor)
                .into_iter()
                .rev()
                .filter(|dir| !dir.is_empty())
                .find_map(find)
            {
                return Some(idx);
            }
        }
        rows.iter().position(|row| !row.placeholder)
    }

    /// Move the cursor `delta` rows (clamped) over `rows`, skipping
    /// placeholder rows.
    pub fn move_cursor(&mut self, rows: &[ExplorerRow], delta: isize) {
        let Some(current) = self.cursor_index(rows) else {
            return;
        };
        let last = rows.len() - 1;
        let target = current.saturating_add_signed(delta).min(last);
        let forward = delta >= 0;
        let pick = |from: usize, fwd: bool| -> Option<usize> {
            if fwd {
                (from..=last).find(|idx| !rows[*idx].placeholder)
            } else {
                (0..=from).rev().find(|idx| !rows[*idx].placeholder)
            }
        };
        if let Some(idx) = pick(target, forward).or_else(|| pick(target, !forward)) {
            self.cursor = Some(rows[idx].rel.clone());
        }
    }

    /// Parent folder of the cursor path, or `None` at a root-level entry
    /// (there is no root row) or with no cursor.
    pub fn parent_dir_of_cursor(&self) -> Option<String> {
        let cursor = self.cursor.as_deref()?;
        let parent = parent_rel(cursor);
        (!parent.is_empty()).then(|| parent.to_string())
    }

    /// Move the cursor to its parent folder row (`h` on a file, `-`).
    /// Returns false and leaves the cursor at a root-level entry.
    pub fn cursor_to_parent(&mut self) -> bool {
        match self.parent_dir_of_cursor() {
            Some(parent) => {
                self.cursor = Some(parent);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{git, init_repo, unique_dir};
    use std::fs;

    fn entry(name: &str, is_dir: bool) -> ExplorerEntry {
        ExplorerEntry {
            name: name.to_string(),
            is_dir,
            ignored: false,
        }
    }

    fn change(path: &str, staged: Option<&str>, unstaged: Option<&str>) -> FileChange {
        FileChange {
            path: path.to_string(),
            staged_status: staged.map(str::to_string),
            unstaged_status: unstaged.map(str::to_string),
            untracked: false,
            old_path: None,
        }
    }

    fn rels(rows: &[ExplorerRow]) -> Vec<(String, usize)> {
        rows.iter()
            .map(|row| (row.rel.clone(), row.depth))
            .collect()
    }

    fn names(entries: &[ExplorerEntry]) -> Vec<(&str, bool, bool)> {
        entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.is_dir, entry.ignored))
            .collect()
    }

    #[test]
    fn list_dir_sorts_folders_first_case_insensitive_and_skips_git() {
        let cwd = unique_dir("ws-explorer-sort");
        init_repo(&cwd);
        fs::create_dir_all(cwd.join("beta")).unwrap();
        fs::create_dir_all(cwd.join("Alpha")).unwrap();
        fs::write(cwd.join("zeta.rs"), "").unwrap();
        fs::write(cwd.join("b.txt"), "").unwrap();
        fs::write(cwd.join("B.txt"), "").unwrap();
        fs::write(cwd.join("a.md"), "").unwrap();

        let listed = list_dir(&cwd, "", false).unwrap();
        assert_eq!(
            names(&listed),
            vec![
                ("Alpha", true, false),
                ("beta", true, false),
                ("a.md", false, false),
                ("B.txt", false, false),
                ("b.txt", false, false),
                ("README.md", false, false),
                ("zeta.rs", false, false),
            ]
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn list_dir_flags_gitignored_entries_and_keeps_tracked_files() {
        let cwd = unique_dir("ws-explorer-ignore");
        init_repo(&cwd);
        fs::write(cwd.join(".gitignore"), "node_modules/\ntarget/\n*.log\n").unwrap();
        fs::write(cwd.join("kept.log"), "tracked\n").unwrap();
        git(&cwd, &["add", "-f", ".gitignore", "kept.log"]);
        git(&cwd, &["commit", "-q", "-m", "ignore"]);
        fs::create_dir_all(cwd.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(cwd.join("target")).unwrap();
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/app.log"), "").unwrap();
        fs::write(cwd.join("src/main.rs"), "").unwrap();
        fs::write(cwd.join("debug.log"), "").unwrap();

        let root = list_dir(&cwd, "", false).unwrap();
        assert_eq!(
            names(&root),
            vec![
                ("node_modules", true, true),
                ("src", true, false),
                ("target", true, true),
                (".gitignore", false, false),
                ("debug.log", false, true),
                ("kept.log", false, false),
                ("README.md", false, false),
            ]
        );
        let src = list_dir(&cwd, "src", false).unwrap();
        assert_eq!(
            names(&src),
            vec![("app.log", false, true), ("main.rs", false, false)]
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    /// A root-level name that starts with `:` must not parse as pathspec
    /// magic: check-ignore rejects `:!` (exclude) with a fatal error, which
    /// would leave every entry un-ignored. `:` is not a legal Windows file
    /// name character.
    #[cfg(unix)]
    #[test]
    fn list_dir_checks_a_leading_colon_name_literally() {
        let cwd = unique_dir("ws-explorer-colon");
        init_repo(&cwd);
        fs::write(cwd.join(".gitignore"), "*.log\n").unwrap();
        fs::write(cwd.join(":!odd.log"), "").unwrap();
        fs::write(cwd.join("plain.log"), "").unwrap();

        let root = list_dir(&cwd, "", false).unwrap();
        assert_eq!(
            names(&root),
            vec![
                (".gitignore", false, false),
                (":!odd.log", false, true),
                ("plain.log", false, true),
                ("README.md", false, false),
            ]
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn children_of_an_ignored_dir_are_ignored_without_git() {
        // Not a git repository: a git call would fail and leave entries
        // un-ignored, so `ignored: true` proves git was not asked.
        let cwd = unique_dir("ws-explorer-parent-ignored");
        fs::create_dir_all(cwd.join("node_modules/pkg")).unwrap();
        fs::write(cwd.join("node_modules/index.js"), "").unwrap();

        let listed = list_dir(&cwd, "node_modules", true).unwrap();
        assert_eq!(
            names(&listed),
            vec![("pkg", true, true), ("index.js", false, true)]
        );
        let plain = list_dir(&cwd, "node_modules", false).unwrap();
        assert!(plain.iter().all(|entry| !entry.ignored));
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn tree_reports_ignored_dir_from_parent_listing() {
        let mut tree = ExplorerTree::new();
        tree.expand("");
        tree.apply_listing(
            "",
            vec![
                ExplorerEntry {
                    name: "target".into(),
                    is_dir: true,
                    ignored: true,
                },
                entry("src", true),
            ],
        );
        assert!(tree.is_ignored_dir("target"));
        assert!(!tree.is_ignored_dir("src"));
        assert!(!tree.is_ignored_dir(""));
    }

    #[test]
    fn rows_flatten_with_depth_and_lazy_expand_requests_once() {
        let status = ExplorerStatus::default();
        let mut tree = ExplorerTree::new();
        assert_eq!(tree.expand(""), Some(String::new()));
        assert_eq!(tree.expand(""), None, "root already requested");
        let loading = tree.rows(&status);
        assert_eq!(loading.len(), 1);
        assert!(loading[0].placeholder);
        assert_eq!(tree.cursor_index(&loading), None);

        tree.apply_listing("", vec![entry("src", true), entry("README.md", false)]);
        assert_eq!(tree.expand("src"), Some("src".to_string()));
        assert_eq!(tree.expand("src"), None, "request in flight");

        let rows = tree.rows(&status);
        assert_eq!(
            rels(&rows),
            vec![
                ("src".into(), 0),
                ("src".into(), 1),
                ("README.md".into(), 0)
            ]
        );
        assert!(rows[0].expanded && rows[1].placeholder);

        tree.apply_listing("src", vec![entry("tui", true), entry("lib.rs", false)]);
        let rows = tree.rows(&status);
        assert_eq!(
            rels(&rows),
            vec![
                ("src".into(), 0),
                ("src/tui".into(), 1),
                ("src/lib.rs".into(), 1),
                ("README.md".into(), 0),
            ]
        );
        assert!(!rows[1].expanded);

        tree.collapse("src");
        assert_eq!(tree.toggle("src"), None, "listing cached");
        assert_eq!(tree.toggle("src"), None);
        assert_eq!(rels(&tree.rows(&status)).len(), 2);
    }

    #[test]
    fn cursor_skips_placeholders_and_follows_collapse() {
        let status = ExplorerStatus::default();
        let mut tree = ExplorerTree::new();
        tree.expand("");
        tree.apply_listing("", vec![entry("a", true), entry("z.rs", false)]);
        tree.expand("a");
        let rows = tree.rows(&status);
        assert_eq!(tree.cursor_index(&rows), Some(0));
        tree.move_cursor(&rows, 1);
        assert_eq!(tree.cursor_rel(), Some("z.rs"), "placeholder skipped");
        tree.move_cursor(&rows, -1);
        assert_eq!(tree.cursor_rel(), Some("a"));
        tree.move_cursor(&rows, 99);
        assert_eq!(tree.cursor_rel(), Some("z.rs"));

        tree.apply_listing("a", vec![entry("b.rs", false)]);
        tree.set_cursor_rel("a/b.rs");
        tree.collapse("a");
        assert_eq!(tree.cursor_rel(), Some("a"));
    }

    #[test]
    fn reveal_expands_ancestors_and_returns_missing_root_first() {
        let status = ExplorerStatus::default();
        let mut tree = ExplorerTree::new();
        tree.expand("");
        tree.apply_listing("", vec![entry("src", true)]);
        assert_eq!(
            tree.reveal("src/tui/app.rs", false),
            vec!["src".to_string(), "src/tui".to_string()]
        );
        assert_eq!(tree.cursor_rel(), Some("src/tui/app.rs"));
        assert!(tree.is_expanded("src") && tree.is_expanded("src/tui"));

        tree.apply_listing("src", vec![entry("tui", true)]);
        tree.apply_listing("src/tui", vec![entry("deep", true), entry("app.rs", false)]);
        let rows = tree.rows(&status);
        assert_eq!(tree.cursor_index(&rows), Some(3));

        assert_eq!(
            tree.reveal("src/tui/deep", true),
            vec!["src/tui/deep".to_string()]
        );
        assert_eq!(tree.cursor_rel(), Some("src/tui/deep"));
    }

    #[test]
    fn cursor_to_parent_moves_up_and_stops_at_root_level() {
        let mut tree = ExplorerTree::new();
        tree.set_cursor_rel("src/tui/app.rs");
        assert_eq!(tree.parent_dir_of_cursor(), Some("src/tui".to_string()));
        assert!(tree.cursor_to_parent());
        assert_eq!(tree.cursor_rel(), Some("src/tui"));
        assert!(tree.cursor_to_parent());
        assert_eq!(tree.cursor_rel(), Some("src"));
        assert!(!tree.cursor_to_parent());
        assert_eq!(tree.cursor_rel(), Some("src"));
    }

    #[test]
    fn status_letters_and_nested_dirty_dirs() {
        let mut untracked = change("docs/new/guide.md", None, None);
        untracked.untracked = true;
        let status = ExplorerStatus::from_changes(&[
            change("src/tui/app.rs", None, Some("M")),
            change("src/added.rs", Some("A"), None),
            change("old.rs", None, Some("D")),
            untracked,
        ]);
        assert_eq!(status.letter("src/tui/app.rs"), Some(FileStatusLetter::M));
        assert_eq!(status.letter("src/added.rs"), Some(FileStatusLetter::A));
        assert_eq!(status.letter("old.rs"), Some(FileStatusLetter::D));
        // Untracked files use the Workspace tree's letter (A).
        assert_eq!(
            status.letter("docs/new/guide.md"),
            Some(FileStatusLetter::A)
        );
        assert_eq!(status.letter("src/lib.rs"), None);
        assert!(status.is_untracked("docs/new/guide.md"));
        assert!(!status.is_untracked("src/added.rs"));
        assert!(!status.is_untracked("docs/new"));

        assert!(status.dir_dirty("src"));
        assert!(status.dir_dirty("src/tui"));
        assert!(status.dir_dirty("docs/new"));
        assert!(!status.dir_dirty("src/tu"));
        assert!(!status.dir_dirty("assets"));
        assert_eq!(status.deleted_in(""), ["old.rs".to_string()]);
        assert!(status.deleted_in("src").is_empty());
    }

    #[test]
    fn staged_then_deleted_files_count_as_deleted() {
        let status = ExplorerStatus::from_changes(&[
            change("src/edited.rs", Some("M"), Some("D")),
            change("src/added.rs", Some("A"), Some("D")),
            change("src/kept.rs", Some("M"), Some("M")),
        ]);
        assert_eq!(
            status.deleted_in("src"),
            ["edited.rs".to_string(), "added.rs".to_string()]
        );
        let mut tree = ExplorerTree::new();
        tree.expand("");
        tree.apply_listing("", vec![entry("src", true)]);
        tree.expand("src");
        tree.apply_listing("src", vec![entry("kept.rs", false)]);
        assert_eq!(
            rels(&tree.rows(&status)),
            vec![
                ("src".into(), 0),
                ("src/added.rs".into(), 1),
                ("src/edited.rs".into(), 1),
                ("src/kept.rs".into(), 1),
            ]
        );
    }

    #[test]
    fn deleted_file_is_a_row_though_not_on_disk() {
        let status = ExplorerStatus::from_changes(&[
            change("src/gone.rs", None, Some("D")),
            change("src/kept.rs", Some("D"), None),
        ]);
        let mut tree = ExplorerTree::new();
        tree.expand("");
        tree.apply_listing("", vec![entry("src", true)]);
        tree.expand("src");
        // `kept.rs` is on disk (`git rm --cached`): listed once, not twice.
        tree.apply_listing(
            "src",
            vec![
                entry("lib", true),
                entry("kept.rs", false),
                entry("z.rs", false),
            ],
        );
        let rows = tree.rows(&status);
        assert_eq!(
            rels(&rows),
            vec![
                ("src".into(), 0),
                ("src/lib".into(), 1),
                ("src/gone.rs".into(), 1),
                ("src/kept.rs".into(), 1),
                ("src/z.rs".into(), 1),
            ]
        );
        assert!(!rows[2].is_dir && !rows[2].ignored && !rows[2].placeholder);
    }

    #[test]
    fn cursor_stays_on_same_rel_after_listing_reorders() {
        let status = ExplorerStatus::default();
        let mut tree = ExplorerTree::new();
        tree.expand("");
        tree.apply_listing("", vec![entry("b.rs", false), entry("c.rs", false)]);
        tree.set_cursor_rel("c.rs");
        assert_eq!(tree.cursor_index(&tree.rows(&status)), Some(1));

        tree.apply_listing(
            "",
            vec![
                entry("a", true),
                entry("a.rs", false),
                entry("b.rs", false),
                entry("c.rs", false),
            ],
        );
        assert_eq!(tree.cursor_rel(), Some("c.rs"));
        assert_eq!(tree.cursor_index(&tree.rows(&status)), Some(3));

        tree.apply_listing("", vec![entry("a.rs", false)]);
        assert_eq!(
            tree.cursor_index(&tree.rows(&status)),
            Some(0),
            "gone: first row"
        );
    }

    #[test]
    fn invalidate_returns_expanded_loaded_dirs_and_drops_collapsed() {
        let mut tree = ExplorerTree::new();
        tree.expand("");
        tree.apply_listing("", vec![entry("a", true), entry("b", true)]);
        tree.expand("a");
        tree.apply_listing("a", vec![entry("deep", true)]);
        tree.expand("a/deep");
        tree.apply_listing("a/deep", vec![]);
        tree.expand("b");
        tree.apply_listing("b", vec![]);
        tree.collapse("b");

        assert_eq!(
            tree.invalidate(),
            vec!["".to_string(), "a".to_string(), "a/deep".to_string()]
        );
        // `a/deep` stays expanded but is hidden while `a` is collapsed.
        tree.collapse("a");
        assert_eq!(tree.invalidate(), vec!["".to_string()]);
        assert_eq!(
            tree.expand("a"),
            Some("a".to_string()),
            "collapsed `a` reloads"
        );
        assert_eq!(
            tree.expand("a/deep"),
            None,
            "hidden `a/deep` kept its listing"
        );
        tree.apply_listing("a", vec![entry("deep", true)]);
        assert_eq!(
            tree.invalidate(),
            vec!["".to_string(), "a".to_string(), "a/deep".to_string()]
        );
        assert_eq!(
            tree.expand("b"),
            Some("b".to_string()),
            "collapsed listing dropped"
        );
    }
}

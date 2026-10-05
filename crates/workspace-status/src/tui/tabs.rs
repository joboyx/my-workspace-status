//! Session tab strip: permanent Workspace plus compare and file tabs.
//!
//! Compare identity is `(checkout_path, base_ref, head_ref, worktree_file)`
//! (`worktree_file` only on a commit-vs-working-tree tab); file identity
//! is `(checkout, rel)`. Both kinds share one strip in creation order and
//! one id counter. Tabs are session-only.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::file_index::FileRead;

use super::diff::DiffContent;
use super::drill::{CommitFile, CommitFileSource};

/// Palette / overlay copy when the focused row is not a compare target.
pub const FOCUS_A_CHECKOUT: &str = "Focus a checkout to compare";
/// Palette copy when HEAD is unborn.
pub const HEAD_HAS_NO_COMMIT: &str = "HEAD has no commit";
/// Palette copy when the default tip ref does not exist.
pub const DEFAULT_BRANCH_NOT_FOUND: &str = "Default branch not found";
/// Diff commit vs parent refusal when the graph pane does not focus a commit.
pub const FOCUS_A_COMMIT_TO_DIFF: &str = "focus a commit to diff";
/// Diff commit vs parent refusal on a commit with no parent.
pub const ROOT_COMMIT_HAS_NO_PARENT: &str = "root commit has no parent";
/// Compare `x` refusal on a tab with a pinned head: the diff is history.
pub const CANNOT_REVERT_COMMITTED_DIFF: &str = "cannot revert a committed diff";
/// Compare `x` refusal on a commit-vs-working-tree tab.
pub const CANNOT_REVERT_WORKTREE_COMPARE: &str = "cannot revert a working-tree compare";
/// Space refusal on a commit-vs-working-tree tab: a mark keys on a range.
pub const REVIEWED_MARKS_NEED_A_COMMIT_RANGE: &str = "reviewed marks need a commit range";
/// Comment, reference, export, and external-diff refusal on a
/// commit-vs-working-tree tab.
pub const NOT_ON_WORKTREE_COMPARE: &str = "not available on a working-tree compare";
/// Palette copy when Close is run on the Workspace tab.
pub const WORKSPACE_TAB_CANNOT_CLOSE: &str = "Workspace tab cannot be closed";

/// `gt` / `gT` and the Next / Previous tab palette rows with no compare tab.
pub const ONLY_WORKSPACE_TAB_OPEN: &str = "only the Workspace tab is open";
/// Mutation disable copy on a compare or file tab.
pub const SWITCH_TO_WORKSPACE_TAB: &str = "Switch to Workspace tab";
/// File tab body for a file with a NUL byte near the start.
pub const FILE_IS_BINARY: &str = "binary file — e opens it in the editor";

/// File tab body for a file over the 2 MiB read cap:
/// `file is over 2 MiB (X.Y MiB) — e opens it in the editor`.
pub fn file_too_large(bytes: u64) -> String {
    let mib = bytes as f64 / (1024.0 * 1024.0);
    format!("file is over 2 MiB ({mib:.1} MiB) — e opens it in the editor")
}
/// Stage disable copy on a compare tab (whole file or highlighted lines).
pub const CANNOT_STAGE_COMPARE: &str = "cannot stage a compare diff";
/// Unstage disable copy on a compare tab (whole file or highlighted lines).
pub const CANNOT_UNSTAGE_COMPARE: &str = "cannot unstage a compare diff";
/// Compare `x` refusal while the range or the open diff is still loading.
pub const COMPARE_STILL_LOADING: &str = "compare diff still loading";
/// Compare `x` refusal when the checkout HEAD is not the loaded compare head.
pub const COMPARE_HEAD_MOVED: &str = "compare is stale: HEAD moved";
/// Compare `x` refusal on a directory row or an empty list.
pub const COMPARE_FOCUS_A_FILE: &str = "focus a file to revert";

/// Compare `x` refusal when `path` differs from HEAD in the worktree or index.
pub fn compare_file_dirty(path: &str) -> String {
    format!("{path} has uncommitted changes")
}

/// Empty compare file list.
pub const NO_COMMITTED_CHANGES: &str = "No committed changes";
/// Empty commit-vs-working-tree file list: the file on disk equals the
/// commit.
pub const NO_CHANGES: &str = "No changes";
/// Empty compare picker.
pub const NO_BRANCHES_TO_COMPARE: &str = "No branches to compare";
/// Empty compare commit picker: HEAD has no ancestor (a root commit).
pub const NO_COMMITS_TO_COMPARE: &str = "No commits to compare";

/// Compare picker copy when no row shows: no rows at all, or none that
/// match the typed filter (`no branch matches <q>` / `no commit matches <q>`).
pub fn compare_picker_empty(picker: &ComparePickerState) -> String {
    match picker {
        ComparePickerState::Branch(picker) if picker.branches.is_empty() => {
            NO_BRANCHES_TO_COMPARE.to_string()
        }
        ComparePickerState::Branch(picker) => format!("no branch matches {}", picker.filter),
        ComparePickerState::Commit(picker) if picker.commits.is_empty() => {
            NO_COMMITS_TO_COMPARE.to_string()
        }
        ComparePickerState::Commit(picker) => format!("no commit matches {}", picker.filter),
    }
}

/// What the compare picker lists: Diff vs branch or Diff vs commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparePickerKind {
    /// Local + `origin/*` branches.
    Branch,
    /// HEAD's ancestors, HEAD excluded.
    Commit,
}

/// Rows a compare picker job loaded, by [`ComparePickerKind`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComparePickerRows {
    /// Branch rows for Diff vs branch.
    Branches(Vec<crate::git::LocalBranch>),
    /// Ancestor rows for Diff vs commit.
    Commits(Vec<crate::git::AncestorCommit>),
}

/// The one compare picker overlay (`InputMode::ComparePicker`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComparePickerState {
    /// Diff vs branch: picks a branch name as the base.
    Branch(super::branches::BranchPickerState),
    /// Diff vs commit: picks an ancestor's full id as the base.
    Commit(CommitPickerState),
}

impl ComparePickerState {
    /// Checkout the picked base compares against.
    pub fn repo(&self) -> &str {
        match self {
            Self::Branch(picker) => &picker.repo,
            Self::Commit(picker) => &picker.repo,
        }
    }

    /// Typed filter.
    pub fn filter(&self) -> &str {
        match self {
            Self::Branch(picker) => &picker.filter,
            Self::Commit(picker) => &picker.filter,
        }
    }

    /// Highlighted row index into the visible rows.
    pub fn cursor(&self) -> usize {
        match self {
            Self::Branch(picker) => picker.cursor,
            Self::Commit(picker) => picker.cursor,
        }
    }

    /// Painted text of `count` matching rows from `start`: `<name>` or
    /// `<short sha>  <subject>`. Only the painted window is formatted.
    pub fn window_labels(&self, start: usize, count: usize) -> Vec<String> {
        match self {
            Self::Branch(picker) => picker
                .visible()
                .into_iter()
                .skip(start)
                .take(count)
                .map(|branch| branch.name.clone())
                .collect(),
            Self::Commit(picker) => picker
                .visible()
                .into_iter()
                .skip(start)
                .take(count)
                .map(|commit| format!("{}  {}", short_rev(&commit.id), commit.subject))
                .collect(),
        }
    }

    /// Count of rows that match the filter.
    pub fn visible_len(&self) -> usize {
        match self {
            Self::Branch(picker) => picker.visible().len(),
            Self::Commit(picker) => picker.visible().len(),
        }
    }

    /// Base ref under the cursor: a branch name or a full commit id.
    pub fn selected_base(&self) -> Option<String> {
        match self {
            Self::Branch(picker) => picker.selected().map(|branch| branch.name.clone()),
            Self::Commit(picker) => picker.selected().map(|commit| commit.id.clone()),
        }
    }

    /// Move the cursor by `delta` rows, clamped to the visible rows.
    pub fn move_cursor(&mut self, delta: i32) {
        match self {
            Self::Branch(picker) => picker.move_cursor(delta),
            Self::Commit(picker) => picker.move_cursor(delta),
        }
    }

    /// Replace the filter and clamp the cursor to the new rows.
    pub fn set_filter(&mut self, filter: String) {
        match self {
            Self::Branch(picker) => picker.set_filter(filter),
            Self::Commit(picker) => picker.set_filter(filter),
        }
    }
}

/// Diff vs commit picker: HEAD's ancestors, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitPickerState {
    /// Checkout whose HEAD the ancestors belong to.
    pub repo: String,
    /// Ancestors from `git log`, HEAD excluded.
    pub commits: Vec<crate::git::AncestorCommit>,
    /// Typed filter.
    pub filter: String,
    /// Index into [`Self::visible`].
    pub cursor: usize,
    /// Lowercased `<id>\n<subject>` per commit, built once for the filter.
    search: Vec<String>,
}

impl CommitPickerState {
    /// Picker over `commits` with an empty filter.
    pub fn new(repo: String, commits: Vec<crate::git::AncestorCommit>) -> Self {
        let search = commits
            .iter()
            .map(|commit| format!("{}\n{}", commit.id, commit.subject).to_lowercase())
            .collect();
        Self {
            repo,
            commits,
            filter: String::new(),
            cursor: 0,
            search,
        }
    }

    /// Commits whose full id, short id, or subject contains the trimmed
    /// filter, ignoring case. A short id is a prefix of the full id, so the
    /// full id covers both.
    pub fn visible(&self) -> Vec<&crate::git::AncestorCommit> {
        let query = self.filter.trim().to_lowercase();
        if query.is_empty() {
            return self.commits.iter().collect();
        }
        self.commits
            .iter()
            .zip(&self.search)
            .filter(|(_, haystack)| haystack.contains(&query))
            .map(|(commit, _)| commit)
            .collect()
    }

    /// Commit under the cursor.
    pub fn selected(&self) -> Option<&crate::git::AncestorCommit> {
        self.visible().get(self.cursor).copied()
    }

    /// Move the cursor by `delta` rows, clamped to the visible rows.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as i32 + delta;
        self.cursor = next.clamp(0, len as i32 - 1) as usize;
    }

    /// Replace the filter and clamp the cursor to the new rows.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        let len = self.visible().len();
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }
}

/// Status after Esc closes the compare picker.
pub const COMPARE_CANCELLED: &str = "compare cancelled";

/// Right-pane empty copy for equal or behind tips.
pub fn no_committed_changes_vs(base_ref: &str) -> String {
    format!("No committed changes vs {}", short_rev(base_ref))
}

/// Right-pane empty copy for a commit-vs-working-tree tab whose file on
/// disk equals the commit.
pub fn no_changes_vs(base_ref: &str) -> String {
    format!("No changes vs {}", short_rev(base_ref))
}

/// Missing base after the tab already exists.
pub fn base_ref_not_found(base_ref: &str) -> String {
    format!("Base ref not found: {}", short_rev(base_ref))
}

/// Pinned head that no longer resolves after the tab already exists.
pub fn head_ref_not_found(head_ref: &str) -> String {
    format!("Head ref not found: {}", short_rev(head_ref))
}

/// Unrelated histories after the tab already exists.
pub fn no_merge_base(base_ref: &str, head_ref: &str) -> String {
    format!(
        "No merge base between {} and {}",
        short_rev(base_ref),
        short_rev(head_ref)
    )
}

/// Compare head of a live tab: the checkout's HEAD at each load.
///
/// Any other [`CompareTab::head_ref`] is a full commit id that pins the head.
pub const COMPARE_HEAD_REF: &str = "HEAD";

/// Display form of a rev: a leading 40- or 64-hex object id shortens to 7
/// chars and keeps any suffix (`<sha>^` → `abc1234^`). Branch names and
/// `HEAD` pass through.
pub fn short_rev(rev: &str) -> String {
    let hex = rev.bytes().take_while(u8::is_ascii_hexdigit).count();
    if hex == 40 || hex == 64 {
        format!("{}{}", &rev[..7], &rev[hex..])
    } else {
        rev.to_string()
    }
}

/// Range copy for chrome and status: `<short base>...<short head>`.
///
/// A live tab reads `main...HEAD`; a pinned tab `abc1234^...abc1234`.
pub fn compare_range_label(base_ref: &str, head_ref: &str) -> String {
    format!("{}...{}", short_rev(base_ref), short_rev(head_ref))
}

/// Bidirectional separator between checkout leaf and base ref (VS Code-style).
pub const COMPARE_TAB_SEP: &str = " ↔ ";

/// Tab strip label: `<checkout-leaf> ↔ <short base-ref>`.
///
/// Left is the checkout leaf; right is the compare base ref (see
/// [`short_rev`]). Order is fixed.
pub fn compare_tab_label(checkout_path: &str, base_ref: &str) -> String {
    format!(
        "{}{COMPARE_TAB_SEP}{}",
        checkout_leaf(checkout_path),
        short_rev(base_ref)
    )
}

/// The one file a commit-vs-working-tree compare tab diffs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeFile {
    /// Path on disk, relative to the checkout.
    pub path: String,
    /// Path at the base commit when it differs from [`Self::path`].
    pub old_path: Option<String>,
}

/// Last path component of a checkout path.
pub fn checkout_leaf(checkout_path: &str) -> String {
    Path::new(checkout_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(checkout_path)
        .to_string()
}

/// One session-only compare tab.
#[derive(Clone, Debug)]
pub struct CompareTab {
    /// Stable id for in-flight jobs.
    pub id: u64,
    /// Checkout path (same string as snapshot `repo`).
    pub checkout_path: String,
    /// Picker / default tip ref, or `<sha>^`. Tab identity with
    /// [`Self::checkout_path`] and [`Self::head_ref`].
    pub base_ref: String,
    /// [`COMPARE_HEAD_REF`] for a live tab, else the full commit id that
    /// pins the head (Diff commit vs parent).
    pub head_ref: String,
    /// Set on a commit-vs-working-tree tab: [`Self::base_ref`] (a full
    /// commit id) against this file on disk. Part of the tab identity, so
    /// it never shares a tab with `<sha>...HEAD`. [`Self::head_ref`] stays
    /// [`COMPARE_HEAD_REF`].
    pub worktree_file: Option<WorktreeFile>,
    /// Commit-vs-working-tree only: the checkout's HEAD and the file's
    /// workspace-snapshot entry when the diff last loaded. A watch tick
    /// that sees another stamp reloads the diff.
    pub worktree_stamp: Option<String>,
    /// Immutable endpoints for the current load, when resolved.
    pub source: Option<CommitFileSource>,
    /// Load or probe error. Tab stays open.
    pub error: Option<String>,
    pub files: Vec<CommitFile>,
    pub file_cursor: usize,
    pub path: Option<String>,
    pub content: DiffContent,
    /// Range and path that [`Self::content`] was loaded for.
    ///
    /// [`Self::path`] moves when a diff load is queued, before the content
    /// arrives, so a write that reads `content` must check this matches.
    pub content_for: Option<(CommitFileSource, String)>,
    /// Left file list when false; right diff when true.
    pub focus_right: bool,
    pub folds: HashSet<String>,
    /// Commit-file directory tree vs flat paths. Independent of Workspace.
    pub tree_mode: bool,
    pub left_col_offset: u16,
    pub diff_col_offset: u16,
    pub diff_cursor: usize,
    pub diff_scroll: u16,
    pub search_mode: bool,
    pub search_active: bool,
    pub search_query: String,
    pub search_hit: Option<usize>,
    pub generation: u64,
    pub last_head: Option<String>,
    pub last_base_tip: Option<String>,
    pub loading: bool,
    /// Latest `LoadCompareDiff` request for this tab.
    pub diff_req: u64,
    /// Session-only reviewed marks: path → range key at mark time.
    ///
    /// A mark paints only while that key matches the loaded range. Stale
    /// entries stay hidden, not removed.
    pub reviewed: HashMap<String, String>,
}

impl CompareTab {
    fn new(
        id: u64,
        checkout_path: String,
        base_ref: String,
        head_ref: String,
        worktree_file: Option<WorktreeFile>,
        tree_mode: bool,
    ) -> Self {
        Self {
            id,
            checkout_path,
            base_ref,
            head_ref,
            worktree_file,
            worktree_stamp: None,
            source: None,
            error: None,
            files: Vec::new(),
            file_cursor: 0,
            path: None,
            content: DiffContent::default(),
            content_for: None,
            focus_right: false,
            folds: HashSet::new(),
            tree_mode,
            left_col_offset: 0,
            diff_col_offset: 0,
            diff_cursor: 0,
            diff_scroll: 0,
            search_mode: false,
            search_active: false,
            search_query: String::new(),
            search_hit: None,
            generation: 0,
            last_head: None,
            last_base_tip: None,
            loading: true,
            diff_req: 0,
            reviewed: HashMap::new(),
        }
    }

    /// Strip label.
    pub fn label(&self) -> String {
        compare_tab_label(&self.checkout_path, &self.base_ref)
    }

    /// Bump the load generation and mark the tab loading.
    pub fn bump_generation(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.loading = true;
        self.generation
    }

    /// True when the head is a fixed commit, not the checkout's HEAD.
    pub fn is_pinned(&self) -> bool {
        self.head_ref != COMPARE_HEAD_REF
    }

    /// Diff pane header range (`<base-ref>...HEAD`,
    /// `abc1234^...abc1234` for a pinned head, or
    /// `abc1234 ↔ working tree` for a commit-vs-working-tree tab).
    pub fn range_header(&self) -> String {
        if self.worktree_file.is_some() {
            return format!("{}{COMPARE_TAB_SEP}working tree", short_rev(&self.base_ref));
        }
        compare_range_label(&self.base_ref, &self.head_ref)
    }

    /// File list copy when the loaded range lists no file.
    pub fn empty_files_copy(&self) -> &'static str {
        if self.worktree_file.is_some() {
            NO_CHANGES
        } else {
            NO_COMMITTED_CHANGES
        }
    }

    /// Diff pane copy when the loaded range lists no file.
    pub fn empty_diff_copy(&self) -> String {
        if self.worktree_file.is_some() {
            no_changes_vs(&self.base_ref)
        } else {
            no_committed_changes_vs(&self.base_ref)
        }
    }

    /// Path of the open diff when [`Self::content`] is loaded for the
    /// current range and [`Self::path`], else `None` (still loading).
    pub fn loaded_diff_path(&self) -> Option<&str> {
        if self.loading {
            return None;
        }
        let (source, path) = self.content_for.as_ref()?;
        (self.source.as_ref() == Some(source) && self.path.as_deref() == Some(path.as_str()))
            .then_some(path.as_str())
    }

    /// Range key of the loaded list when `path` is in it, else `None`.
    ///
    /// Merge base + HEAD. A new HEAD or merge base changes it, so a mark
    /// stops painting. A base tip that moves without a new merge base keeps it.
    fn reviewed_range(&self, path: &str) -> Option<String> {
        let Some(CommitFileSource::Compare {
            merge_base, head, ..
        }) = self.source.as_ref()
        else {
            return None;
        };
        self.files
            .iter()
            .any(|file| file.path == path)
            .then(|| format!("{merge_base}\0{head}"))
    }

    /// True when `path` is marked reviewed for the loaded range.
    pub fn is_reviewed(&self, path: &str) -> bool {
        let Some(marked) = self.reviewed.get(path) else {
            return false;
        };
        self.reviewed_range(path).as_ref() == Some(marked)
    }

    /// Toggle the reviewed mark on `path`. No-op when `path` is not listed.
    pub fn toggle_reviewed(&mut self, path: &str) {
        let Some(now) = self.reviewed_range(path) else {
            return;
        };
        if self.reviewed.get(path) == Some(&now) {
            self.reviewed.remove(path);
        } else {
            self.reviewed.insert(path.to_string(), now);
        }
    }
}

/// Columns of the file tab line-number gutter: the digits of
/// `line_count` plus one blank separator column.
pub fn file_gutter_width(line_count: usize) -> usize {
    line_count.max(1).to_string().len() + 1
}

/// One session-only read-only file tab.
#[derive(Clone, Debug)]
pub struct FileTab {
    /// Stable id for in-flight loads (shared counter with compare tabs).
    pub id: u64,
    /// Checkout path (same string as snapshot `repo`).
    pub checkout: String,
    /// Path relative to [`Self::checkout`]. Identity with the checkout.
    pub rel: String,
    /// Pane title: `<checkout>/<rel>` (snapshot `repo` path, unique per
    /// checkout).
    pub display: String,
    /// Loaded body. `None` while a load is in flight.
    pub body: Option<Arc<FileRead>>,
    /// Load generation. A result for an older generation is dropped.
    pub generation: u64,
    /// Focused 0-based line.
    pub cursor: usize,
    /// First painted line.
    pub scroll: usize,
    /// Horizontal pan of the code columns.
    pub col_offset: u16,
    /// Parked `/` typing state while another tab is active.
    pub search_mode: bool,
    /// Parked armed-search flag.
    pub search_active: bool,
    /// Parked search query.
    pub search_query: String,
    /// Line of the last search match.
    pub search_hit: Option<usize>,
}

impl FileTab {
    fn new(id: u64, checkout: String, rel: String, display: String) -> Self {
        Self {
            id,
            checkout,
            rel,
            display,
            body: None,
            generation: 0,
            cursor: 0,
            scroll: 0,
            col_offset: 0,
            search_mode: false,
            search_active: false,
            search_query: String::new(),
            search_hit: None,
        }
    }

    /// Strip label: the file name of [`Self::rel`].
    pub fn label(&self) -> String {
        checkout_leaf(&self.rel)
    }

    /// Painted lines. Empty unless the body loaded as text.
    pub fn lines(&self) -> &[String] {
        match self.body.as_deref() {
            Some(FileRead::Text { lines, .. }) => lines,
            _ => &[],
        }
    }

    /// Bump the load generation and drop the body (loading again).
    pub fn bump_generation(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.body = None;
        self.generation
    }
}

/// One tab after Workspace: a compare tab or a file tab.
#[derive(Clone, Debug)]
pub enum SessionTab {
    /// Committed-range compare tab.
    Compare(CompareTab),
    /// Read-only file viewer tab.
    File(FileTab),
}

impl SessionTab {
    /// Strip label.
    pub fn label(&self) -> String {
        match self {
            Self::Compare(tab) => tab.label(),
            Self::File(tab) => tab.label(),
        }
    }

    /// The compare tab, if this is one.
    pub fn as_compare(&self) -> Option<&CompareTab> {
        match self {
            Self::Compare(tab) => Some(tab),
            Self::File(_) => None,
        }
    }

    /// Mutable [`Self::as_compare`].
    pub fn as_compare_mut(&mut self) -> Option<&mut CompareTab> {
        match self {
            Self::Compare(tab) => Some(tab),
            Self::File(_) => None,
        }
    }

    /// The file tab, if this is one.
    pub fn as_file(&self) -> Option<&FileTab> {
        match self {
            Self::File(tab) => Some(tab),
            Self::Compare(_) => None,
        }
    }

    /// Mutable [`Self::as_file`].
    pub fn as_file_mut(&mut self) -> Option<&mut FileTab> {
        match self {
            Self::File(tab) => Some(tab),
            Self::Compare(_) => None,
        }
    }
}

/// Permanent Workspace plus compare and file tabs in creation order.
#[derive(Clone, Debug)]
pub struct TabStrip {
    /// 0 is Workspace. Session tabs follow.
    pub active: usize,
    tabs: Vec<SessionTab>,
    /// Commit-file list mode a new compare tab opens in: directory tree
    /// (`true`) or flat paths. The launch `viewDefaults.commitTree`, else
    /// `true`. The `t` toggle changes one tab only, never this default.
    pub commit_tree_default: bool,
    next_id: u64,
}

impl Default for TabStrip {
    fn default() -> Self {
        Self {
            active: 0,
            tabs: Vec::new(),
            commit_tree_default: true,
            next_id: 1,
        }
    }
}

impl TabStrip {
    /// Tab count including Workspace.
    pub fn len(&self) -> usize {
        self.tabs.len() + 1
    }

    /// True when the Workspace tab is active.
    pub fn is_workspace(&self) -> bool {
        self.active == 0
    }

    fn active_tab(&self) -> Option<&SessionTab> {
        self.active.checked_sub(1).and_then(|i| self.tabs.get(i))
    }

    fn active_tab_mut(&mut self) -> Option<&mut SessionTab> {
        self.active
            .checked_sub(1)
            .and_then(|i| self.tabs.get_mut(i))
    }

    /// Active compare tab, if any.
    pub fn active_compare(&self) -> Option<&CompareTab> {
        self.active_tab().and_then(SessionTab::as_compare)
    }

    /// Mutable active compare tab, if any.
    pub fn active_compare_mut(&mut self) -> Option<&mut CompareTab> {
        self.active_tab_mut().and_then(SessionTab::as_compare_mut)
    }

    /// Active file tab, if any.
    pub fn active_file(&self) -> Option<&FileTab> {
        self.active_tab().and_then(SessionTab::as_file)
    }

    /// Mutable active file tab, if any.
    pub fn active_file_mut(&mut self) -> Option<&mut FileTab> {
        self.active_tab_mut().and_then(SessionTab::as_file_mut)
    }

    /// Every compare tab in strip order.
    pub fn compare_tabs(&self) -> impl Iterator<Item = &CompareTab> {
        self.tabs.iter().filter_map(SessionTab::as_compare)
    }

    #[cfg(test)]
    /// Count of open compare tabs.
    pub fn compare_count(&self) -> usize {
        self.compare_tabs().count()
    }

    /// Strip labels in paint order.
    pub fn labels(&self) -> Vec<String> {
        let mut out = vec!["Workspace".to_string()];
        out.extend(self.tabs.iter().map(SessionTab::label));
        out
    }

    #[cfg(test)]
    /// Find a commit-range compare tab by identity. `0` is never returned
    /// (Workspace). A commit-vs-working-tree tab never matches.
    pub fn find(&self, checkout_path: &str, base_ref: &str, head_ref: &str) -> Option<usize> {
        self.find_compare(checkout_path, base_ref, head_ref, None)
    }

    /// Find a compare tab by its full identity, `worktree_file` included.
    fn find_compare(
        &self,
        checkout_path: &str,
        base_ref: &str,
        head_ref: &str,
        worktree_file: Option<&WorktreeFile>,
    ) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| {
                tab.as_compare().is_some_and(|tab| {
                    tab.checkout_path == checkout_path
                        && tab.base_ref == base_ref
                        && tab.head_ref == head_ref
                        && tab.worktree_file.as_ref() == worktree_file
                })
            })
            .map(|i| i + 1)
    }

    /// Find a file tab by identity. `0` is never returned (Workspace).
    pub fn find_file(&self, checkout: &str, rel: &str) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| {
                tab.as_file()
                    .is_some_and(|tab| tab.checkout == checkout && tab.rel == rel)
            })
            .map(|i| i + 1)
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Focus an existing identity or append a new commit-range compare tab.
    pub fn open_or_focus(
        &mut self,
        checkout_path: String,
        base_ref: String,
        head_ref: String,
    ) -> OpenCompare {
        self.open_or_focus_compare(checkout_path, base_ref, head_ref, None)
    }

    /// Focus or append the commit-vs-working-tree tab for commit `base`
    /// (a full id) and `file` in `checkout_path`.
    pub fn open_or_focus_worktree(
        &mut self,
        checkout_path: String,
        base: String,
        file: WorktreeFile,
    ) -> OpenCompare {
        self.open_or_focus_compare(checkout_path, base, COMPARE_HEAD_REF.into(), Some(file))
    }

    fn open_or_focus_compare(
        &mut self,
        checkout_path: String,
        base_ref: String,
        head_ref: String,
        worktree_file: Option<WorktreeFile>,
    ) -> OpenCompare {
        if let Some(index) =
            self.find_compare(&checkout_path, &base_ref, &head_ref, worktree_file.as_ref())
        {
            self.active = index;
            return OpenCompare::Focused;
        }
        let id = self.take_id();
        self.tabs.push(SessionTab::Compare(CompareTab::new(
            id,
            checkout_path,
            base_ref,
            head_ref,
            worktree_file,
            self.commit_tree_default,
        )));
        self.active = self.tabs.len();
        OpenCompare::Created(id)
    }

    /// Focus the file tab for `(checkout, rel)` or append a new one that
    /// paints `display` as its pane title.
    pub fn open_or_focus_file(
        &mut self,
        checkout: String,
        rel: String,
        display: String,
    ) -> OpenFile {
        if let Some(index) = self.find_file(&checkout, &rel) {
            self.active = index;
            return OpenFile::Focused;
        }
        let id = self.take_id();
        self.tabs
            .push(SessionTab::File(FileTab::new(id, checkout, rel, display)));
        self.active = self.tabs.len();
        OpenFile::Created(id)
    }

    #[cfg(test)]
    /// Close the active session tab. Workspace is a no-op.
    ///
    /// Activates the tab immediately to the left.
    pub fn close_active(&mut self) -> bool {
        self.close_at(self.active)
    }

    /// Close the compare or file tab at strip `index`. `0` (Workspace) is a
    /// no-op.
    ///
    /// Closing the active tab activates the tab immediately to the left.
    /// Closing a tab left of the active tab shifts `active` down by one.
    pub fn close_at(&mut self, index: usize) -> bool {
        let Some(tab_i) = index.checked_sub(1) else {
            return false;
        };
        if tab_i >= self.tabs.len() {
            return false;
        }
        self.tabs.remove(tab_i);
        match self.active.cmp(&index) {
            std::cmp::Ordering::Equal => self.active = tab_i,
            std::cmp::Ordering::Greater => self.active -= 1,
            std::cmp::Ordering::Less => {}
        }
        true
    }

    #[cfg(test)]
    /// Next tab, wrapping.
    pub fn next(&mut self) {
        if self.len() == 0 {
            return;
        }
        self.active = (self.active + 1) % self.len();
    }

    #[cfg(test)]
    /// Previous tab, wrapping.
    pub fn prev(&mut self) {
        let len = self.len();
        if len == 0 {
            return;
        }
        self.active = if self.active == 0 {
            len - 1
        } else {
            self.active - 1
        };
    }

    #[cfg(test)]
    /// Absolute `g1`…`g9`. Missing index is a silent no-op. `1` is Workspace.
    pub fn jump(&mut self, n: u8) {
        if n == 0 {
            return;
        }
        let index = n as usize - 1;
        if index < self.len() {
            self.active = index;
        }
    }

    /// Compare tab by stable id.
    pub fn get_id(&self, id: u64) -> Option<&CompareTab> {
        self.compare_tabs().find(|tab| tab.id == id)
    }

    /// Mutable compare tab by stable id.
    pub fn get_id_mut(&mut self, id: u64) -> Option<&mut CompareTab> {
        self.tabs
            .iter_mut()
            .filter_map(SessionTab::as_compare_mut)
            .find(|tab| tab.id == id)
    }

    /// Mutable file tab by stable id.
    pub fn get_file_id_mut(&mut self, id: u64) -> Option<&mut FileTab> {
        self.tabs
            .iter_mut()
            .filter_map(SessionTab::as_file_mut)
            .find(|tab| tab.id == id)
    }
}

/// Result of [`TabStrip::open_or_focus_file`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenFile {
    /// Existing identity. State is unchanged.
    Focused,
    /// New tab that still needs a load.
    Created(u64),
}

/// Result of [`TabStrip::open_or_focus`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenCompare {
    /// Existing identity. State is unchanged.
    Focused,
    /// New tab that still needs a range load.
    Created(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_reopen_focuses_without_reset() {
        let mut tabs = TabStrip::default();
        assert_eq!(
            tabs.open_or_focus("app".into(), "origin/main".into(), "HEAD".into()),
            OpenCompare::Created(1)
        );
        tabs.active_compare_mut().unwrap().file_cursor = 3;
        assert_eq!(
            tabs.open_or_focus("app".into(), "origin/main".into(), "HEAD".into()),
            OpenCompare::Focused
        );
        assert_eq!(tabs.active, 1);
        assert_eq!(tabs.active_compare().unwrap().file_cursor, 3);
    }

    #[test]
    fn close_activates_left_and_workspace_never_closes() {
        let mut tabs = TabStrip::default();
        tabs.open_or_focus("app".into(), "origin/main".into(), "HEAD".into());
        tabs.open_or_focus("app".into(), "develop".into(), "HEAD".into());
        assert_eq!(tabs.active, 2);
        assert!(tabs.close_active());
        assert_eq!(tabs.active, 1);
        assert!(tabs.close_active());
        assert!(tabs.is_workspace());
        assert!(!tabs.close_active());
        assert!(tabs.is_workspace());
    }

    #[test]
    fn jump_and_wrap() {
        let mut tabs = TabStrip::default();
        tabs.open_or_focus("app".into(), "a".into(), "HEAD".into());
        tabs.open_or_focus("app".into(), "b".into(), "HEAD".into());
        tabs.jump(1);
        assert!(tabs.is_workspace());
        tabs.jump(9);
        assert!(tabs.is_workspace());
        tabs.next();
        assert_eq!(tabs.active, 1);
        tabs.prev();
        assert!(tabs.is_workspace());
        tabs.prev();
        assert_eq!(tabs.active, 2);
        assert_eq!(tabs.labels()[1], "app ↔ a");
    }

    #[test]
    fn close_at_workspace_is_noop_and_inactive_compare_closes() {
        let mut tabs = TabStrip::default();
        tabs.open_or_focus("app".into(), "a".into(), "HEAD".into());
        tabs.open_or_focus("app".into(), "b".into(), "HEAD".into());
        assert_eq!(tabs.active, 2);
        assert!(!tabs.close_at(0));
        assert_eq!(tabs.active, 2);
        assert_eq!(tabs.compare_count(), 2);
        assert!(tabs.close_at(1));
        assert_eq!(tabs.active, 1);
        assert_eq!(tabs.compare_count(), 1);
        assert_eq!(tabs.labels()[1], "app ↔ b");
    }

    #[test]
    fn file_tab_identity_reopen_focuses() {
        let mut tabs = TabStrip::default();
        let open = |tabs: &mut TabStrip| {
            tabs.open_or_focus_file("app".into(), "src/main.rs".into(), "app/src/main.rs".into())
        };
        assert_eq!(open(&mut tabs), OpenFile::Created(1));
        tabs.active_file_mut().unwrap().cursor = 4;
        tabs.active = 0;
        assert_eq!(open(&mut tabs), OpenFile::Focused);
        assert_eq!(tabs.active, 1);
        assert_eq!(tabs.active_file().unwrap().cursor, 4);
        assert_eq!(tabs.labels()[1], "main.rs");
        assert_eq!(
            tabs.open_or_focus_file("lib".into(), "src/main.rs".into(), "lib/src/main.rs".into()),
            OpenFile::Created(2),
            "same path in another checkout is another tab"
        );
    }

    #[test]
    fn strip_keeps_creation_order_across_kinds() {
        let mut tabs = TabStrip::default();
        tabs.open_or_focus_file("app".into(), "README.md".into(), "app/README.md".into());
        assert_eq!(
            tabs.open_or_focus("app".into(), "main".into(), COMPARE_HEAD_REF.into()),
            OpenCompare::Created(2),
            "one id counter for both kinds"
        );
        assert_eq!(tabs.labels(), vec!["Workspace", "README.md", "app ↔ main"]);
        tabs.active = 1;
        assert!(tabs.active_compare().is_none());
        assert!(tabs.active_file().is_some());
        tabs.active = 2;
        assert!(tabs.active_compare().is_some());
        assert!(tabs.active_file().is_none());
        assert_eq!(tabs.find("app", "main", COMPARE_HEAD_REF), Some(2));
        assert_eq!(
            tabs.get_id(2).map(|tab| tab.base_ref.as_str()),
            Some("main")
        );
        assert!(tabs.get_id(1).is_none(), "id 1 is the file tab");

        tabs.active = 1;
        assert!(tabs.close_at(1));
        assert!(tabs.is_workspace(), "closing the active tab activates left");
        assert_eq!(tabs.labels(), vec!["Workspace", "app ↔ main"]);
        assert_eq!(tabs.compare_count(), 1);
        assert_eq!(tabs.find("app", "main", COMPARE_HEAD_REF), Some(1));
    }

    #[test]
    fn file_tab_lines_and_too_large_copy() {
        let mut tabs = TabStrip::default();
        tabs.open_or_focus_file("app".into(), "a.txt".into(), "app/a.txt".into());
        let tab = tabs.active_file_mut().unwrap();
        assert!(tab.lines().is_empty(), "loading");
        tab.body = Some(Arc::new(FileRead::Text {
            lines: vec!["one".into(), "two".into()],
            max_cols: 3,
        }));
        assert_eq!(tab.lines(), ["one", "two"]);
        assert_eq!(tab.bump_generation(), 1);
        assert!(tab.body.is_none());
        assert_eq!(
            file_too_large(3 * 1024 * 1024 + 512 * 1024),
            "file is over 2 MiB (3.5 MiB) — e opens it in the editor"
        );
        assert_eq!(file_gutter_width(9), 2);
        assert_eq!(file_gutter_width(10), 3);
        assert_eq!(file_gutter_width(0), 2);
    }

    #[test]
    fn compare_tab_label_uses_bidirectional_arrow_not_vs() {
        let label = compare_tab_label("repos/app", "origin/main");
        assert_eq!(label, "app ↔ origin/main");
        assert!(!label.contains("vs"));
        assert!(!label.contains('·'));
        assert!(label.contains(COMPARE_TAB_SEP));
        // Left = checkout leaf, right = base ref (ordering preserved).
        let (left, right) = label.split_once(COMPARE_TAB_SEP).expect("sep");
        assert_eq!(left, "app");
        assert_eq!(right, "origin/main");
    }

    const SHA: &str = "abc1234def5678abc1234def5678abc1234def56";

    #[test]
    fn short_rev_shortens_object_ids_and_keeps_names() {
        assert_eq!(short_rev(SHA), "abc1234");
        assert_eq!(short_rev(&format!("{SHA}^")), "abc1234^");
        let sha256 = "a".repeat(64);
        assert_eq!(short_rev(&sha256), "aaaaaaa");
        assert_eq!(short_rev("origin/main"), "origin/main");
        assert_eq!(short_rev("HEAD"), "HEAD");
        assert_eq!(short_rev("deadbeef"), "deadbeef", "short hex is a name");
        let long = "a".repeat(41);
        assert_eq!(short_rev(&long), long, "41 hex is not an object id");
    }

    #[test]
    fn identity_includes_head_ref() {
        let mut tabs = TabStrip::default();
        let base = format!("{SHA}^");
        assert_eq!(
            tabs.open_or_focus("app".into(), base.clone(), COMPARE_HEAD_REF.into()),
            OpenCompare::Created(1)
        );
        assert_eq!(
            tabs.open_or_focus("app".into(), base.clone(), SHA.into()),
            OpenCompare::Created(2),
            "same base, other head is a second tab"
        );
        tabs.active = 1;
        assert_eq!(
            tabs.open_or_focus("app".into(), base.clone(), SHA.into()),
            OpenCompare::Focused
        );
        assert_eq!(tabs.active, 2);
        assert_eq!(tabs.find("app", &base, SHA), Some(2));
        assert_eq!(tabs.find("app", &base, COMPARE_HEAD_REF), Some(1));
        assert_eq!(tabs.find("app", &base, "other"), None);
    }

    #[test]
    fn pinned_tab_label_range_and_short_label_use_short_shas() {
        let mut tabs = TabStrip::default();
        tabs.open_or_focus("repos/app".into(), format!("{SHA}^"), SHA.into());
        let tab = tabs.active_compare().unwrap();
        assert!(tab.is_pinned());
        assert_eq!(tab.label(), "app ↔ abc1234^");
        assert_eq!(tab.range_header(), "abc1234^...abc1234");
        let source = CommitFileSource::Compare {
            base_ref: format!("{SHA}^"),
            head_ref: SHA.into(),
            base_tip: "b".repeat(40),
            merge_base: "b".repeat(40),
            head: SHA.into(),
        };
        assert_eq!(source.short_label(), "abc1234^...abc1234");
        assert_eq!(source.files_status(2), "2 files in abc1234^...abc1234");

        tabs.open_or_focus("repos/app".into(), "main".into(), COMPARE_HEAD_REF.into());
        let live = tabs.active_compare().unwrap();
        assert!(!live.is_pinned());
        assert_eq!(live.label(), "app ↔ main");
        assert_eq!(live.range_header(), "main...HEAD");
    }

    #[test]
    fn range_copy_shortens_the_base_and_head() {
        let base = format!("{SHA}^");
        assert_eq!(
            no_committed_changes_vs(&base),
            "No committed changes vs abc1234^"
        );
        assert_eq!(base_ref_not_found(&base), "Base ref not found: abc1234^");
        assert_eq!(head_ref_not_found(SHA), "Head ref not found: abc1234");
        assert_eq!(
            no_merge_base("main", COMPARE_HEAD_REF),
            "No merge base between main and HEAD"
        );
        assert_eq!(
            no_merge_base(&base, SHA),
            "No merge base between abc1234^ and abc1234"
        );
    }

    fn loaded_tab(merge_base: &str, head: &str, base_tip: &str) -> CompareTab {
        let mut tab = CompareTab::new(
            1,
            "app".into(),
            "main".into(),
            COMPARE_HEAD_REF.into(),
            None,
            true,
        );
        tab.source = Some(CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: base_tip.into(),
            merge_base: merge_base.into(),
            head: head.into(),
        });
        tab.files = vec![CommitFile {
            status: "M".into(),
            path: "src/a.rs".into(),
            old_path: None,
            stat: None,
        }];
        tab
    }

    #[test]
    fn reviewed_toggles_on_listed_file_only() {
        let mut tab = loaded_tab("aaa", "ccc", "bbb");
        tab.toggle_reviewed("src/missing.rs");
        assert!(tab.reviewed.is_empty());
        tab.toggle_reviewed("src/a.rs");
        assert!(tab.is_reviewed("src/a.rs"));
        tab.toggle_reviewed("src/a.rs");
        assert!(!tab.is_reviewed("src/a.rs"));
    }

    #[test]
    fn reviewed_survives_base_tip_move_but_not_new_head() {
        let mut tab = loaded_tab("aaa", "ccc", "bbb");
        tab.toggle_reviewed("src/a.rs");
        let marks = tab.reviewed.clone();

        let mut moved_tip = loaded_tab("aaa", "ccc", "bbb2");
        moved_tip.reviewed = marks.clone();
        assert!(moved_tip.is_reviewed("src/a.rs"));

        let mut new_head = loaded_tab("aaa", "ddd", "bbb");
        new_head.reviewed = marks;
        assert!(!new_head.is_reviewed("src/a.rs"));
        new_head.toggle_reviewed("src/a.rs");
        assert!(new_head.is_reviewed("src/a.rs"), "stale mark re-marks");
    }

    #[test]
    fn worktree_tab_identity_never_meets_a_sha_head_tab() {
        let mut tabs = TabStrip::default();
        let file = |path: &str| WorktreeFile {
            path: path.into(),
            old_path: None,
        };
        assert_eq!(
            tabs.open_or_focus("app".into(), SHA.into(), COMPARE_HEAD_REF.into()),
            OpenCompare::Created(1)
        );
        assert_eq!(
            tabs.open_or_focus_worktree("app".into(), SHA.into(), file("a.rs")),
            OpenCompare::Created(2)
        );
        assert_eq!(
            tabs.open_or_focus_worktree("app".into(), SHA.into(), file("b.rs")),
            OpenCompare::Created(3)
        );
        assert_eq!(
            tabs.open_or_focus_worktree("app".into(), SHA.into(), file("a.rs")),
            OpenCompare::Focused
        );
        assert_eq!(tabs.active, 2);
        assert_eq!(tabs.find("app", SHA, COMPARE_HEAD_REF), Some(1));
        assert_eq!(
            tabs.open_or_focus("app".into(), SHA.into(), COMPARE_HEAD_REF.into()),
            OpenCompare::Focused
        );
        assert_eq!(tabs.active, 1);

        let live = tabs.get_id(1).unwrap();
        let worktree = tabs.get_id(2).unwrap();
        assert_eq!(live.range_header(), "abc1234...HEAD");
        assert_eq!(worktree.range_header(), "abc1234 ↔ working tree");
        assert_eq!(worktree.label(), "app ↔ abc1234");
        assert!(!worktree.is_pinned());
        assert_eq!(live.empty_files_copy(), NO_COMMITTED_CHANGES);
        assert_eq!(worktree.empty_files_copy(), NO_CHANGES);
        assert_eq!(live.empty_diff_copy(), "No committed changes vs abc1234");
        assert_eq!(worktree.empty_diff_copy(), "No changes vs abc1234");
    }
}

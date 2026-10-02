//! Session tab strip: permanent Workspace plus compare tabs.
//!
//! Compare identity is `(checkout_path, base_ref, head_ref)`. Tabs are
//! session-only.

use std::collections::{HashMap, HashSet};
use std::path::Path;

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
/// Palette copy when Close is run on the Workspace tab.
pub const WORKSPACE_TAB_CANNOT_CLOSE: &str = "Workspace tab cannot be closed";

/// `gt` / `gT` and the Next / Previous tab palette rows with no compare tab.
pub const ONLY_WORKSPACE_TAB_OPEN: &str = "only the Workspace tab is open";
/// Mutation disable copy on a compare tab.
pub const SWITCH_TO_WORKSPACE_TAB: &str = "Switch to Workspace tab";
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
/// Empty compare picker.
pub const NO_BRANCHES_TO_COMPARE: &str = "No branches to compare";

/// Compare picker copy when no row shows: no branches at all, or none
/// that match the typed filter (`no branch matches <q>`).
pub fn compare_picker_empty(picker: &super::branches::BranchPickerState) -> String {
    if picker.branches.is_empty() {
        NO_BRANCHES_TO_COMPARE.to_string()
    } else {
        format!("no branch matches {}", picker.filter)
    }
}

/// Status after Esc closes the compare picker.
pub const COMPARE_CANCELLED: &str = "compare cancelled";

/// Right-pane empty copy for equal or behind tips.
pub fn no_committed_changes_vs(base_ref: &str) -> String {
    format!("No committed changes vs {}", short_rev(base_ref))
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
    fn new(id: u64, checkout_path: String, base_ref: String, head_ref: String) -> Self {
        Self {
            id,
            checkout_path,
            base_ref,
            head_ref,
            source: None,
            error: None,
            files: Vec::new(),
            file_cursor: 0,
            path: None,
            content: DiffContent::default(),
            content_for: None,
            focus_right: false,
            folds: HashSet::new(),
            tree_mode: true,
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

    /// Diff pane header range (`<base-ref>...HEAD`, or
    /// `abc1234^...abc1234` for a pinned head).
    pub fn range_header(&self) -> String {
        compare_range_label(&self.base_ref, &self.head_ref)
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

/// Permanent Workspace plus compare tabs in creation order.
#[derive(Clone, Debug)]
pub struct TabStrip {
    /// 0 is Workspace. Compare tabs follow.
    pub active: usize,
    pub compare: Vec<CompareTab>,
    next_id: u64,
}

impl Default for TabStrip {
    fn default() -> Self {
        Self {
            active: 0,
            compare: Vec::new(),
            next_id: 1,
        }
    }
}

impl TabStrip {
    /// Tab count including Workspace.
    pub fn len(&self) -> usize {
        self.compare.len() + 1
    }

    /// True when the Workspace tab is active.
    pub fn is_workspace(&self) -> bool {
        self.active == 0
    }

    /// Active compare tab, if any.
    pub fn active_compare(&self) -> Option<&CompareTab> {
        self.active.checked_sub(1).and_then(|i| self.compare.get(i))
    }

    /// Mutable active compare tab, if any.
    pub fn active_compare_mut(&mut self) -> Option<&mut CompareTab> {
        self.active
            .checked_sub(1)
            .and_then(|i| self.compare.get_mut(i))
    }

    /// Strip labels in paint order.
    pub fn labels(&self) -> Vec<String> {
        let mut out = vec!["Workspace".to_string()];
        out.extend(self.compare.iter().map(CompareTab::label));
        out
    }

    /// Find a tab by identity. `0` is never returned (Workspace).
    pub fn find(&self, checkout_path: &str, base_ref: &str, head_ref: &str) -> Option<usize> {
        self.compare
            .iter()
            .position(|tab| {
                tab.checkout_path == checkout_path
                    && tab.base_ref == base_ref
                    && tab.head_ref == head_ref
            })
            .map(|i| i + 1)
    }

    /// Focus an existing identity or append a new compare tab.
    pub fn open_or_focus(
        &mut self,
        checkout_path: String,
        base_ref: String,
        head_ref: String,
    ) -> OpenCompare {
        if let Some(index) = self.find(&checkout_path, &base_ref, &head_ref) {
            self.active = index;
            return OpenCompare::Focused;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.compare
            .push(CompareTab::new(id, checkout_path, base_ref, head_ref));
        self.active = self.compare.len();
        OpenCompare::Created(id)
    }

    #[cfg(test)]
    /// Close the active compare tab. Workspace is a no-op.
    ///
    /// Activates the tab immediately to the left.
    pub fn close_active_compare(&mut self) -> bool {
        self.close_at(self.active)
    }

    /// Close the compare tab at strip `index`. `0` (Workspace) is a no-op.
    ///
    /// Closing the active tab activates the tab immediately to the left.
    /// Closing a tab left of the active tab shifts `active` down by one.
    pub fn close_at(&mut self, index: usize) -> bool {
        let Some(cmp_i) = index.checked_sub(1) else {
            return false;
        };
        if cmp_i >= self.compare.len() {
            return false;
        }
        self.compare.remove(cmp_i);
        match self.active.cmp(&index) {
            std::cmp::Ordering::Equal => self.active = cmp_i,
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
        self.compare.iter().find(|tab| tab.id == id)
    }

    /// Mutable compare tab by stable id.
    pub fn get_id_mut(&mut self, id: u64) -> Option<&mut CompareTab> {
        self.compare.iter_mut().find(|tab| tab.id == id)
    }
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
        assert!(tabs.close_active_compare());
        assert_eq!(tabs.active, 1);
        assert!(tabs.close_active_compare());
        assert!(tabs.is_workspace());
        assert!(!tabs.close_active_compare());
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
        assert_eq!(tabs.compare.len(), 2);
        assert!(tabs.close_at(1));
        assert_eq!(tabs.active, 1);
        assert_eq!(tabs.compare.len(), 1);
        assert_eq!(tabs.labels()[1], "app ↔ b");
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
        let mut tab = CompareTab::new(1, "app".into(), "main".into(), COMPARE_HEAD_REF.into());
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
}

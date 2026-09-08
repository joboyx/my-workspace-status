//! Session tab strip: permanent Workspace plus compare tabs.
//!
//! Compare identity is `(checkout_path, base_ref)`. Tabs are session-only.

use std::collections::HashSet;
use std::path::Path;

use super::diff::DiffContent;
use super::drill::{CommitFile, CommitFileSource};

/// Palette / overlay copy when the focused row is not a compare target.
pub const FOCUS_A_CHECKOUT: &str = "Focus a checkout to compare";
/// Palette copy when HEAD is unborn.
pub const HEAD_HAS_NO_COMMIT: &str = "HEAD has no commit";
/// Palette copy when the default tip ref does not exist.
pub const DEFAULT_BRANCH_NOT_FOUND: &str = "Default branch not found";
/// Palette copy when Close is run on the Workspace tab.
pub const WORKSPACE_TAB_CANNOT_CLOSE: &str = "Workspace tab cannot be closed";
/// Mutation disable copy on a compare tab.
pub const SWITCH_TO_WORKSPACE_TAB: &str = "Switch to Workspace tab";
/// Empty compare file list.
pub const NO_COMMITTED_CHANGES: &str = "No committed changes";
/// Empty compare picker.
pub const NO_BRANCHES_TO_COMPARE: &str = "No branches to compare";

/// Right-pane empty copy for equal or behind tips.
pub fn no_committed_changes_vs(base_ref: &str) -> String {
    format!("No committed changes vs {base_ref}")
}

/// Missing base after the tab already exists.
pub fn base_ref_not_found(base_ref: &str) -> String {
    format!("Base ref not found: {base_ref}")
}

/// Unrelated histories after the tab already exists.
pub fn no_merge_base(base_ref: &str) -> String {
    format!("No merge base between {base_ref} and HEAD")
}

/// Tab strip label: `<checkout-leaf> · vs <base-ref>`.
pub fn compare_tab_label(checkout_path: &str, base_ref: &str) -> String {
    format!("{} · vs {base_ref}", checkout_leaf(checkout_path))
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
    /// Picker / default tip ref. Tab identity with [`Self::checkout_path`].
    pub base_ref: String,
    /// Immutable endpoints for the current load, when resolved.
    pub source: Option<CommitFileSource>,
    /// Load or probe error. Tab stays open.
    pub error: Option<String>,
    pub files: Vec<CommitFile>,
    pub file_cursor: usize,
    pub path: Option<String>,
    pub content: DiffContent,
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
}

impl CompareTab {
    fn new(id: u64, checkout_path: String, base_ref: String) -> Self {
        Self {
            id,
            checkout_path,
            base_ref,
            source: None,
            error: None,
            files: Vec::new(),
            file_cursor: 0,
            path: None,
            content: DiffContent::default(),
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

    /// Diff pane header range (`<base-ref>...HEAD`).
    pub fn range_header(&self) -> String {
        format!("{}...HEAD", self.base_ref)
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
    pub fn find(&self, checkout_path: &str, base_ref: &str) -> Option<usize> {
        self.compare
            .iter()
            .position(|tab| tab.checkout_path == checkout_path && tab.base_ref == base_ref)
            .map(|i| i + 1)
    }

    /// Focus an existing identity or append a new compare tab.
    pub fn open_or_focus(&mut self, checkout_path: String, base_ref: String) -> OpenCompare {
        if let Some(index) = self.find(&checkout_path, &base_ref) {
            self.active = index;
            return OpenCompare::Focused;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.compare
            .push(CompareTab::new(id, checkout_path, base_ref));
        self.active = self.compare.len();
        OpenCompare::Created(id)
    }

    /// Close the active compare tab. Workspace is a no-op.
    ///
    /// Activates the tab immediately to the left.
    pub fn close_active_compare(&mut self) -> bool {
        let Some(index) = self.active.checked_sub(1) else {
            return false;
        };
        if index >= self.compare.len() {
            return false;
        }
        self.compare.remove(index);
        self.active = index;
        true
    }

    /// Next tab, wrapping.
    pub fn next(&mut self) {
        if self.len() == 0 {
            return;
        }
        self.active = (self.active + 1) % self.len();
    }

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
            tabs.open_or_focus("app".into(), "origin/main".into()),
            OpenCompare::Created(1)
        );
        tabs.active_compare_mut().unwrap().file_cursor = 3;
        assert_eq!(
            tabs.open_or_focus("app".into(), "origin/main".into()),
            OpenCompare::Focused
        );
        assert_eq!(tabs.active, 1);
        assert_eq!(tabs.active_compare().unwrap().file_cursor, 3);
    }

    #[test]
    fn close_activates_left_and_workspace_never_closes() {
        let mut tabs = TabStrip::default();
        tabs.open_or_focus("app".into(), "origin/main".into());
        tabs.open_or_focus("app".into(), "develop".into());
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
        tabs.open_or_focus("app".into(), "a".into());
        tabs.open_or_focus("app".into(), "b".into());
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
        assert_eq!(tabs.labels()[1], "app · vs a");
    }
}

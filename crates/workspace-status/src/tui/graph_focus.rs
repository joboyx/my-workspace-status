//! Graph branch-focus overlay (`o` / `O`).
//!
//! Lists local branches. Space marks a set; Enter applies visible marks, or
//! the cursor row when none of the visible rows are marked. Marks hidden by
//! the filter (including the current focus pre-marked on reopen) do not leak
//! through. The graph then loads ancestors of those tips instead of `--all`.
//! Unmarking every `[x]` then Enter, and `O`, restore the full graph.

use std::collections::BTreeSet;

use crate::git::LocalBranch;

use super::branches::filter_branches;

/// Interactive overlay for choosing graph focus branches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphFocusPickerState {
    /// Checkout the graph was loaded for.
    pub repo: String,
    /// Local branches, default-first then newest.
    pub branches: Vec<LocalBranch>,
    /// Substring filter (case-insensitive).
    pub filter: String,
    /// Index into [`Self::visible`].
    pub cursor: usize,
    /// Names toggled with space. Enter uses this set for rows still visible
    /// under the filter.
    pub marked: BTreeSet<String>,
    /// Enter with no remaining marks restores `--all` when the user cleared
    /// every `[x]` (including a pre-marked current focus). Filter-then-Enter
    /// still applies the cursor row while any mark remains, including hidden.
    empty_apply_clears: bool,
}

impl GraphFocusPickerState {
    /// Build a picker. `preselected` names that still exist start marked.
    pub fn new(repo: String, branches: Vec<LocalBranch>, preselected: &[String]) -> Self {
        let names: BTreeSet<String> = branches.iter().map(|b| b.name.clone()).collect();
        let marked: BTreeSet<String> = preselected
            .iter()
            .filter(|name| names.contains(*name))
            .cloned()
            .collect();
        let cursor = preselected
            .iter()
            .find(|name| names.contains(*name))
            .and_then(|first| {
                filter_branches(&branches, "")
                    .iter()
                    .position(|branch| branch.name == *first)
            })
            .unwrap_or(0);
        Self {
            repo,
            branches,
            filter: String::new(),
            cursor,
            empty_apply_clears: !marked.is_empty(),
            marked,
        }
    }

    /// Branches matching the current filter.
    pub fn visible(&self) -> Vec<&LocalBranch> {
        filter_branches(&self.branches, &self.filter)
    }

    /// Cursor row, if the filter still has hits.
    pub fn selected(&self) -> Option<&LocalBranch> {
        let visible = self.visible();
        visible.get(self.cursor).copied()
    }

    /// Move the cursor in the filtered list.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as i32 + delta;
        self.cursor = next.clamp(0, len as i32 - 1) as usize;
    }

    /// Replace the filter and clamp the cursor.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(len - 1);
        }
    }

    /// Toggle space-mark on the cursor row.
    pub fn toggle_mark(&mut self) {
        let Some(name) = self.selected().map(|branch| branch.name.clone()) else {
            return;
        };
        self.empty_apply_clears = true;
        if !self.marked.remove(&name) {
            self.marked.insert(name);
        }
    }

    /// Names to load, or `None` when the filter has no rows (Enter is a no-op).
    ///
    /// Visible space-marks win. Hidden marks do not leak through a filter.
    /// When no visible row is marked, Enter applies the cursor row, except
    /// after every `[x]` was cleared (`Some([])` → restore `--all`).
    pub fn apply_names(&self) -> Option<Vec<String>> {
        let visible = self.visible();
        if visible.is_empty() {
            return None;
        }
        let visible_marks: Vec<String> = visible
            .iter()
            .filter(|branch| self.marked.contains(&branch.name))
            .map(|branch| branch.name.clone())
            .collect();
        if !visible_marks.is_empty() {
            return Some(visible_marks);
        }
        if self.marked.is_empty() && self.empty_apply_clears {
            return Some(Vec::new());
        }
        Some(vec![self.selected()?.name.clone()])
    }
}

/// `git log` revision for a local branch name (`refs/heads/…`).
pub fn focus_rev_for_branch(name: &str) -> String {
    if name.starts_with("refs/") {
        name.to_string()
    } else {
        format!("refs/heads/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(name: &str) -> LocalBranch {
        LocalBranch {
            name: name.into(),
            current: false,
            authordate: 0,
        }
    }

    #[test]
    fn enter_without_marks_uses_cursor_row() {
        let picker =
            GraphFocusPickerState::new("app".into(), vec![b("main"), b("feature/keep")], &[]);
        assert_eq!(picker.apply_names(), Some(vec!["main".into()]));
    }

    #[test]
    fn space_marks_win_over_cursor() {
        let mut picker = GraphFocusPickerState::new(
            "app".into(),
            vec![b("main"), b("feature/keep"), b("topic/noise")],
            &[],
        );
        picker.move_cursor(1);
        picker.toggle_mark();
        picker.move_cursor(1);
        picker.toggle_mark();
        assert_eq!(
            picker.apply_names(),
            Some(vec!["feature/keep".into(), "topic/noise".into()])
        );
    }

    #[test]
    fn filter_then_enter_applies_the_hit() {
        let mut picker = GraphFocusPickerState::new(
            "app".into(),
            vec![b("main"), b("feature/keep"), b("topic/noise")],
            &[],
        );
        picker.set_filter("keep".into());
        assert_eq!(picker.apply_names(), Some(vec!["feature/keep".into()]));
    }

    #[test]
    fn filter_then_enter_replaces_hidden_preselection() {
        let mut picker = GraphFocusPickerState::new(
            "app".into(),
            vec![b("main"), b("feature/keep"), b("topic/noise")],
            &["feature/keep".into()],
        );
        picker.set_filter("noise".into());
        assert_eq!(
            picker.apply_names(),
            Some(vec!["topic/noise".into()]),
            "Enter on a filtered hit should apply that row, not hidden pre-marks"
        );
    }

    #[test]
    fn marking_filtered_row_replaces_hidden_preselection() {
        let mut picker = GraphFocusPickerState::new(
            "app".into(),
            vec![b("main"), b("feature/keep"), b("topic/noise")],
            &["feature/keep".into()],
        );
        picker.set_filter("noise".into());
        picker.toggle_mark();
        assert_eq!(
            picker.apply_names(),
            Some(vec!["topic/noise".into()]),
            "space-marking the visible row should apply that row, not union with hidden pre-marks"
        );
    }

    #[test]
    fn enter_with_visible_preselection_keeps_marks() {
        let picker = GraphFocusPickerState::new(
            "app".into(),
            vec![b("main"), b("feature/keep")],
            &["feature/keep".into()],
        );
        assert_eq!(picker.apply_names(), Some(vec!["feature/keep".into()]));
    }

    #[test]
    fn unmarking_preselection_then_enter_clears() {
        let mut picker = GraphFocusPickerState::new(
            "app".into(),
            vec![b("main"), b("feature/keep"), b("topic/noise")],
            &["feature/keep".into()],
        );
        picker.toggle_mark();
        assert!(picker.marked.is_empty());
        assert_eq!(
            picker.apply_names(),
            Some(Vec::new()),
            "removing every [x] then Enter must restore --all, not the cursor row"
        );
    }

    #[test]
    fn space_on_then_off_without_prior_focus_clears() {
        let mut picker =
            GraphFocusPickerState::new("app".into(), vec![b("main"), b("feature/keep")], &[]);
        picker.toggle_mark();
        picker.toggle_mark();
        assert_eq!(picker.apply_names(), Some(Vec::new()));
    }

    #[test]
    fn empty_filter_hits_are_a_noop() {
        let mut picker =
            GraphFocusPickerState::new("app".into(), vec![b("main"), b("feature/keep")], &[]);
        picker.set_filter("zzz".into());
        assert_eq!(picker.apply_names(), None);
    }

    #[test]
    fn preselected_names_start_marked() {
        let picker = GraphFocusPickerState::new(
            "app".into(),
            vec![b("main"), b("feature/keep")],
            &["feature/keep".into(), "gone".into()],
        );
        assert_eq!(
            picker.marked.iter().cloned().collect::<Vec<_>>(),
            vec!["feature/keep".to_string()]
        );
        assert_eq!(
            picker.selected().map(|b| b.name.as_str()),
            Some("feature/keep")
        );
    }

    #[test]
    fn focus_rev_prefixes_local_names() {
        assert_eq!(
            focus_rev_for_branch("feature/keep"),
            "refs/heads/feature/keep"
        );
        assert_eq!(focus_rev_for_branch("refs/heads/main"), "refs/heads/main");
    }
}

//! Local branch picker (`b`): list, filter, create, checkout.

use crate::git::LocalBranch;
use crate::snapshot::{CheckoutKind, WorkspaceSnapshot};

use super::stash::checkout_path;
use super::tree::{NodeKind, VisibleRow};

/// Default branch name pinned first, then newest authordate.
pub fn sort_branches_for_picker(
    mut branches: Vec<LocalBranch>,
    default_branch: Option<&str>,
) -> Vec<LocalBranch> {
    branches.sort_by(|a, b| {
        if let Some(default) = default_branch {
            let a_default = a.name == default;
            let b_default = b.name == default;
            if a_default != b_default {
                return b_default.cmp(&a_default);
            }
        }
        b.authordate.cmp(&a.authordate).then_with(|| a.name.cmp(&b.name))
    });
    branches
}

/// Case-insensitive substring filter on branch name.
pub fn filter_branches<'a>(branches: &'a [LocalBranch], query: &str) -> Vec<&'a LocalBranch> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return branches.iter().collect();
    }
    branches
        .iter()
        .filter(|b| b.name.to_ascii_lowercase().contains(&q))
        .collect()
}

/// Non-empty, no spaces, no leading `-`.
pub fn is_valid_branch_name(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty() && !t.contains(char::is_whitespace) && !t.starts_with('-')
}

/// True when `b` may open on this row (checkout or flat repo, not a family).
pub fn can_open_branch_picker(snapshot: &WorkspaceSnapshot, row: &VisibleRow) -> bool {
    match row.kind {
        NodeKind::Checkout => checkout_path(row).is_some(),
        NodeKind::Repo => {
            let Some(repo) = row.repo.as_deref() else {
                return false;
            };
            !is_family_container(snapshot, repo)
        }
        NodeKind::File | NodeKind::Workspace | NodeKind::Group => false,
    }
}

fn is_family_container(snapshot: &WorkspaceSnapshot, primary: &str) -> bool {
    snapshot.repos.iter().any(|repo| {
        repo.checkout_kind == CheckoutKind::Linked && repo.primary_repo.as_deref() == Some(primary)
    })
}

/// Interactive picker state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchPickerState {
    pub repo: String,
    pub branches: Vec<LocalBranch>,
    pub filter: String,
    pub cursor: usize,
}

impl BranchPickerState {
    pub fn new(repo: String, branches: Vec<LocalBranch>) -> Self {
        Self {
            repo,
            branches,
            filter: String::new(),
            cursor: 0,
        }
    }

    pub fn visible(&self) -> Vec<&LocalBranch> {
        filter_branches(&self.branches, &self.filter)
    }

    pub fn selected(&self) -> Option<&LocalBranch> {
        let visible = self.visible();
        visible.get(self.cursor).copied()
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as i32 + delta;
        self.cursor = next.clamp(0, len as i32 - 1) as usize;
    }

    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(len - 1);
        }
    }
}

/// Name prompt after `C` in the picker, or Enter on a new filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateBranchState {
    pub repo: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(name: &str, current: bool, authordate: i64) -> LocalBranch {
        LocalBranch {
            name: name.into(),
            current,
            authordate,
        }
    }

    #[test]
    fn default_branch_pins_first_then_newest() {
        let sorted = sort_branches_for_picker(
            vec![
                b("feature/z", false, 30),
                b("main", true, 10),
                b("feature/a", false, 20),
            ],
            Some("main"),
        );
        assert_eq!(
            sorted.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            vec!["main", "feature/z", "feature/a"]
        );
    }

    #[test]
    fn filter_is_case_insensitive() {
        let branches = vec![b("main", true, 1), b("feature/JBY", false, 2)];
        let hits = filter_branches(&branches, "jby");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "feature/JBY");
    }

    #[test]
    fn branch_name_rules() {
        assert!(is_valid_branch_name("feature/x"));
        assert!(!is_valid_branch_name(""));
        assert!(!is_valid_branch_name("has space"));
        assert!(!is_valid_branch_name("-bad"));
    }

    #[test]
    fn picker_cursor_clamps_on_filter() {
        let mut picker = BranchPickerState::new(
            "app".into(),
            vec![b("main", true, 1), b("feat", false, 2)],
        );
        picker.cursor = 1;
        picker.set_filter("main".into());
        assert_eq!(picker.cursor, 0);
        assert_eq!(picker.selected().map(|b| b.name.as_str()), Some("main"));
    }
}

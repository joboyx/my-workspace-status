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

/// True iff `name` is an origin remote-tracking ref (`origin/...`).
pub fn is_origin_remote_ref(name: &str) -> bool {
    name.starts_with("origin/")
}

/// Local short name for an origin remote-tracking ref.
pub fn local_name_from_origin_ref(name: &str) -> &str {
    name.strip_prefix("origin/").unwrap_or(name)
}

/// Local and `origin/*` names `b` may checkout at a graph commit.
///
/// Locals first, then remotes. Each group is unique and sorted.
/// Tags and other remotes stay out when callers pass only those names.
pub fn checkoutable_branch_names(refs: &[String]) -> Vec<String> {
    let mut locals: Vec<String> = refs
        .iter()
        .filter(|name| !is_origin_remote_ref(name))
        .cloned()
        .collect();
    locals.sort();
    locals.dedup();
    let mut remotes: Vec<String> = refs
        .iter()
        .filter(|name| is_origin_remote_ref(name))
        .cloned()
        .collect();
    remotes.sort();
    remotes.dedup();
    locals.extend(remotes);
    locals
}

/// Branch to check out for a picker or single-name selection.
pub fn checkout_name_for_ref(selected: &str) -> String {
    if is_origin_remote_ref(selected) {
        local_name_from_origin_ref(selected).to_string()
    } else {
        selected.to_string()
    }
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
        NodeKind::File | NodeKind::Dir | NodeKind::Workspace | NodeKind::Group => false,
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

    /// Graph `b` picker: only the names on the focused commit.
    pub fn from_names(repo: String, names: Vec<String>) -> Self {
        let branches = names
            .into_iter()
            .map(|name| LocalBranch {
                name,
                current: false,
                authordate: 0,
            })
            .collect();
        Self::new(repo, branches)
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

/// Name prompt after `C` in the picker, graph `c`, or Enter on a new filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateBranchState {
    pub repo: String,
    pub name: String,
    /// Graph `c` sets this. Picker `C` leaves it `None` (create+checkout at HEAD).
    pub commit_id: Option<String>,
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
    fn checkoutable_names_locals_then_origin() {
        let names = checkoutable_branch_names(&[
            "origin/z".into(),
            "topic".into(),
            "main".into(),
            "origin/main".into(),
            "topic".into(),
        ]);
        assert_eq!(names, vec!["main", "topic", "origin/main", "origin/z"]);
        assert_eq!(checkout_name_for_ref("origin/main"), "main");
        assert_eq!(checkout_name_for_ref("feature/x"), "feature/x");
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

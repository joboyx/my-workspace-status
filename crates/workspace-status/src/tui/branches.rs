//! Local branch picker (`b`): list, filter, checkout, and a create row.

use workspace_status_graph::{GraphRef, RefKind};

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
        b.authordate
            .cmp(&a.authordate)
            .then_with(|| a.name.cmp(&b.name))
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

/// Characters `git check-ref-format` refuses anywhere in a ref name.
const BRANCH_NAME_BAD_CHARS: &[char] = &['~', '^', ':', '?', '*', '[', '\\'];

/// Why `name` (trimmed) is not a valid new branch name, or `None` when it is.
///
/// Follows the `git check-ref-format --branch` rules so the create prompt can
/// refuse locally, before git runs: empty, whitespace, a leading `-` or `/`,
/// `..`, `@{`, `//`, a lone `@`, any of `~ ^ : ? * [ \`, control characters,
/// a trailing `/` or `.`, and a `/`-separated part that starts with `.` or
/// ends with `.lock`.
pub fn branch_name_error(name: &str) -> Option<String> {
    let t = name.trim();
    let reason = if t.is_empty() {
        "name is empty".to_string()
    } else if t.contains(char::is_whitespace) {
        "no spaces in a branch name".to_string()
    } else if t.chars().any(char::is_control) {
        "no control characters in a branch name".to_string()
    } else if let Some(c) = t.chars().find(|c| BRANCH_NAME_BAD_CHARS.contains(c)) {
        format!("a branch name cannot contain {c}")
    } else if t.starts_with('-') {
        "a branch name cannot start with -".to_string()
    } else if t == "@" {
        "a branch name cannot be @".to_string()
    } else if let Some(seq) = ["..", "@{", "//"].into_iter().find(|seq| t.contains(seq)) {
        format!("a branch name cannot contain {seq}")
    } else if t.starts_with('/') {
        "a branch name cannot start with /".to_string()
    } else if let Some(end) = ["/", "."].into_iter().find(|end| t.ends_with(end)) {
        format!("a branch name cannot end with {end}")
    } else if t.split('/').any(|part| part.starts_with('.')) {
        "no part of a branch name can start with .".to_string()
    } else if t.split('/').any(|part| part.ends_with(".lock")) {
        "no part of a branch name can end with .lock".to_string()
    } else {
        return None;
    };
    Some(reason)
}

/// True when [`branch_name_error`] finds nothing wrong with `name`.
#[cfg(test)]
pub fn is_valid_branch_name(name: &str) -> bool {
    branch_name_error(name).is_none()
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
/// Tags and non-origin remotes stay out.
pub fn checkoutable_branch_names(refs: &[GraphRef]) -> Vec<String> {
    let mut locals: Vec<String> = refs
        .iter()
        .filter(|graph_ref| graph_ref.kind == RefKind::Local)
        .map(|graph_ref| graph_ref.name.clone())
        .collect();
    locals.sort();
    locals.dedup();
    let mut remotes: Vec<String> = refs
        .iter()
        .filter(|graph_ref| {
            graph_ref.kind == RefKind::Remote && is_origin_remote_ref(&graph_ref.name)
        })
        .map(|graph_ref| graph_ref.name.clone())
        .collect();
    remotes.sort();
    remotes.dedup();
    locals.extend(remotes);
    locals
}

/// Rev and overlay label for merging a focused graph commit into HEAD.
///
/// Local and `origin/*` names are used as-is (same set as checkout). Tags and
/// unlabeled commits use the commit id (short hash as the label).
pub fn merge_rev_for_commit(commit_id: &str, refs: &[GraphRef]) -> (String, String) {
    if let Some(name) = checkoutable_branch_names(refs).into_iter().next() {
        return (name.clone(), name);
    }
    let short = if commit_id.len() >= 7 {
        commit_id[..7].to_string()
    } else {
        commit_id.to_string()
    };
    (commit_id.to_string(), short)
}

/// Branch to check out for a picker or single-name selection.
pub fn checkout_name_for_ref(selected: &str) -> String {
    if is_origin_remote_ref(selected) {
        local_name_from_origin_ref(selected).to_string()
    } else {
        selected.to_string()
    }
}

/// Status copy when checkout refuses a dirty worktree (tracked changes only).
pub const DIRTY_WORKTREE_STATUS: &str = "Dirty worktree — commit or stash first";

/// Pure checkout vs confirm-then-fast-forward decision (no git I/O).
///
/// Origin remotes with an out-of-sync (or unread) local counterpart confirm,
/// then `merge --ff-only` of the selected `origin/*` ref. Local names never
/// confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphCheckoutPlan {
    Checkout {
        branch: String,
    },
    ConfirmLocalThenPull {
        local_branch: String,
        remote_ref: String,
    },
}

/// Plan checkout for a picker or single-name selection.
///
/// Confirm only when `selected_name` is `origin/…` **and** a local branch of
/// that name exists with a null or mismatched SHA.
pub fn plan_graph_checkout(
    selected_name: &str,
    local_exists: bool,
    local_sha: Option<&str>,
    remote_sha: Option<&str>,
) -> GraphCheckoutPlan {
    if !is_origin_remote_ref(selected_name) {
        return GraphCheckoutPlan::Checkout {
            branch: selected_name.to_string(),
        };
    }
    let local_branch = local_name_from_origin_ref(selected_name).to_string();
    if local_exists && (local_sha.is_none() || remote_sha.is_none() || local_sha != remote_sha) {
        return GraphCheckoutPlan::ConfirmLocalThenPull {
            local_branch,
            remote_ref: selected_name.to_string(),
        };
    }
    GraphCheckoutPlan::Checkout {
        branch: local_branch,
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
        NodeKind::File
        | NodeKind::Dir
        | NodeKind::Workspace
        | NodeKind::Group
        | NodeKind::Section => false,
    }
}

/// True when `primary` is a family container (has linked worktrees).
pub fn is_family_container(snapshot: &WorkspaceSnapshot, primary: &str) -> bool {
    snapshot.repos.iter().any(|repo| {
        repo.checkout_kind == CheckoutKind::Linked && repo.primary_repo.as_deref() == Some(primary)
    })
}

/// Interactive picker state.
///
/// The branch (`b`) and graph-checkout pickers end their list with a
/// create row ([`Self::create_name`]); the compare picker does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchPickerState {
    pub repo: String,
    pub branches: Vec<LocalBranch>,
    pub filter: String,
    /// Row index: a visible branch, or the create row right after them.
    pub cursor: usize,
    /// Graph commit picker paints `Checkout at {short}` and creates there.
    /// Tree picker is `None`.
    pub commit_id: Option<String>,
    /// True when a typed name that matches no branch exactly offers a
    /// `+ create branch <name>` row.
    pub creates: bool,
}

impl BranchPickerState {
    /// Picker over `branches` with no create row (the compare picker).
    pub fn new(repo: String, branches: Vec<LocalBranch>) -> Self {
        Self {
            repo,
            branches,
            filter: String::new(),
            cursor: 0,
            commit_id: None,
            creates: false,
        }
    }

    /// Tree `b` picker: checkout, plus a create row that creates the typed
    /// name at HEAD and checks it out.
    pub fn checkout(repo: String, branches: Vec<LocalBranch>) -> Self {
        let mut state = Self::new(repo, branches);
        state.creates = true;
        state
    }

    /// Graph `b` picker: only the names on the focused commit. Its create
    /// row makes the branch at that commit without a checkout.
    pub fn from_names(repo: String, names: Vec<String>, commit_id: Option<String>) -> Self {
        let branches = names
            .into_iter()
            .map(|name| LocalBranch {
                name,
                current: false,
                authordate: 0,
            })
            .collect();
        let mut state = Self::checkout(repo, branches);
        state.commit_id = commit_id;
        state
    }

    /// Branches that match the filter, in list order.
    pub fn visible(&self) -> Vec<&LocalBranch> {
        filter_branches(&self.branches, &self.filter)
    }

    /// Branch under the cursor; `None` on the create row.
    pub fn selected(&self) -> Option<&LocalBranch> {
        let visible = self.visible();
        visible.get(self.cursor).copied()
    }

    /// Name the create row offers, or `None` when the row is hidden.
    ///
    /// Shown when this picker creates, the trimmed filter is non-empty,
    /// it is a valid branch name ([`branch_name_error`]), and no listed
    /// name equals it exactly.
    pub fn create_name(&self) -> Option<&str> {
        let name = self.filter.trim();
        if !self.creates
            || name.is_empty()
            || branch_name_error(name).is_some()
            || self.branches.iter().any(|branch| branch.name == name)
        {
            return None;
        }
        Some(name)
    }

    /// Visible branches plus the create row when it shows.
    pub fn row_count(&self) -> usize {
        self.visible().len() + usize::from(self.create_name().is_some())
    }

    /// True when the cursor sits on the create row.
    pub fn on_create_row(&self) -> bool {
        self.create_name().is_some() && self.cursor == self.visible().len()
    }

    /// Move the cursor by `delta` rows, clamped to the list.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.row_count();
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
        let len = self.row_count();
        if len == 0 {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(len - 1);
        }
    }
}

/// Name prompt after graph `c`: create a branch at a commit, no checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateBranchState {
    pub repo: String,
    pub name: String,
    /// Commit the new branch points at.
    pub commit_id: String,
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
    fn branch_name_error_follows_check_ref_format() {
        let cases: &[(&str, Option<&str>)] = &[
            ("feature/x", None),
            ("JBY-12-fix", None),
            ("  trimmed  ", None),
            ("v1.2", None),
            ("a@b", None),
            ("", Some("name is empty")),
            ("   ", Some("name is empty")),
            ("has space", Some("no spaces in a branch name")),
            ("tab\there", Some("no spaces in a branch name")),
            ("bell\u{7}", Some("no control characters in a branch name")),
            ("-bad", Some("a branch name cannot start with -")),
            ("@", Some("a branch name cannot be @")),
            ("a..b", Some("a branch name cannot contain ..")),
            ("a@{b", Some("a branch name cannot contain @{")),
            ("a//b", Some("a branch name cannot contain //")),
            ("a~1", Some("a branch name cannot contain ~")),
            ("a^", Some("a branch name cannot contain ^")),
            ("a:b", Some("a branch name cannot contain :")),
            ("a?", Some("a branch name cannot contain ?")),
            ("a*", Some("a branch name cannot contain *")),
            ("a[b", Some("a branch name cannot contain [")),
            ("a\\b", Some("a branch name cannot contain \\")),
            ("/lead", Some("a branch name cannot start with /")),
            ("trail/", Some("a branch name cannot end with /")),
            ("trail.", Some("a branch name cannot end with .")),
            (
                "main.lock",
                Some("no part of a branch name can end with .lock"),
            ),
            (
                "x.lock/y",
                Some("no part of a branch name can end with .lock"),
            ),
            (".hidden", Some("no part of a branch name can start with .")),
            (
                "feature/.x",
                Some("no part of a branch name can start with ."),
            ),
        ];
        for (name, want) in cases {
            assert_eq!(branch_name_error(name).as_deref(), *want, "{name:?}");
            assert_eq!(is_valid_branch_name(name), want.is_none(), "{name:?}");
        }
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
    fn plan_local_selection_never_confirms() {
        assert_eq!(
            plan_graph_checkout("main", true, Some("aaa"), Some("bbb")),
            GraphCheckoutPlan::Checkout {
                branch: "main".into()
            }
        );
    }

    #[test]
    fn plan_origin_with_no_local_checkouts_short_name() {
        assert_eq!(
            plan_graph_checkout("origin/feature/x", false, None, Some("abc")),
            GraphCheckoutPlan::Checkout {
                branch: "feature/x".into()
            }
        );
    }

    #[test]
    fn plan_origin_with_local_same_sha_checkouts_short_name() {
        assert_eq!(
            plan_graph_checkout("origin/main", true, Some("aaa"), Some("aaa")),
            GraphCheckoutPlan::Checkout {
                branch: "main".into()
            }
        );
    }

    #[test]
    fn plan_origin_with_local_different_sha_confirms() {
        assert_eq!(
            plan_graph_checkout("origin/main", true, Some("aaa"), Some("bbb")),
            GraphCheckoutPlan::ConfirmLocalThenPull {
                local_branch: "main".into(),
                remote_ref: "origin/main".into(),
            }
        );
    }

    #[test]
    fn plan_origin_with_local_but_a_sha_is_null_confirms() {
        assert_eq!(
            plan_graph_checkout("origin/main", true, Some("aaa"), None),
            GraphCheckoutPlan::ConfirmLocalThenPull {
                local_branch: "main".into(),
                remote_ref: "origin/main".into(),
            }
        );
        assert_eq!(
            plan_graph_checkout("origin/main", true, None, Some("bbb")),
            GraphCheckoutPlan::ConfirmLocalThenPull {
                local_branch: "main".into(),
                remote_ref: "origin/main".into(),
            }
        );
    }

    #[test]
    fn checkoutable_names_skip_tags_and_non_origin() {
        use workspace_status_graph::GraphRef;
        let names = checkoutable_branch_names(&[
            GraphRef::tag("v1.0"),
            GraphRef::remote("upstream/main"),
            GraphRef::local("topic"),
            GraphRef::remote("origin/topic"),
        ]);
        assert_eq!(names, vec!["topic", "origin/topic"]);
    }

    #[test]
    fn merge_rev_prefers_local_then_origin_else_commit() {
        use workspace_status_graph::GraphRef;
        let id = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert_eq!(
            merge_rev_for_commit(
                id,
                &[
                    GraphRef::tag("v1.0"),
                    GraphRef::remote("origin/z"),
                    GraphRef::local("topic"),
                ]
            ),
            ("topic".into(), "topic".into())
        );
        assert_eq!(
            merge_rev_for_commit(id, &[GraphRef::remote("origin/z"), GraphRef::tag("v1.0")]),
            ("origin/z".into(), "origin/z".into())
        );
        assert_eq!(
            merge_rev_for_commit(id, &[GraphRef::tag("v1.0")]),
            (id.into(), "aaa1111".into())
        );
        assert_eq!(merge_rev_for_commit(id, &[]), (id.into(), "aaa1111".into()));
    }

    #[test]
    fn picker_cursor_clamps_on_filter() {
        let mut picker =
            BranchPickerState::new("app".into(), vec![b("main", true, 1), b("feat", false, 2)]);
        picker.cursor = 1;
        picker.set_filter("main".into());
        assert_eq!(picker.cursor, 0);
        assert_eq!(picker.selected().map(|b| b.name.as_str()), Some("main"));
    }

    #[test]
    fn create_row_shows_for_a_new_valid_name_only() {
        let mut picker = BranchPickerState::checkout(
            "app".into(),
            vec![b("main", true, 1), b("feature/x", false, 2)],
        );
        assert_eq!(picker.create_name(), None, "empty filter");
        assert_eq!(picker.row_count(), 2);
        picker.set_filter("feat".into());
        assert_eq!(picker.create_name(), Some("feat"));
        assert_eq!(picker.row_count(), 2, "feature/x + create row");
        picker.move_cursor(5);
        assert!(picker.on_create_row());
        assert_eq!(picker.selected(), None);
        picker.set_filter("feature/x".into());
        assert_eq!(picker.create_name(), None, "exact existing name");
        assert!(!picker.on_create_row());
        picker.set_filter("bad name".into());
        assert_eq!(picker.create_name(), None, "invalid name");
        assert_eq!(picker.row_count(), 0);
        picker.set_filter("  topic  ".into());
        assert_eq!(picker.create_name(), Some("topic"));
        assert!(picker.on_create_row(), "the only row");
    }

    #[test]
    fn compare_picker_has_no_create_row() {
        let mut picker = BranchPickerState::new("app".into(), vec![b("main", true, 1)]);
        picker.set_filter("topic".into());
        assert_eq!(picker.create_name(), None);
        assert_eq!(picker.row_count(), 0);
    }
}

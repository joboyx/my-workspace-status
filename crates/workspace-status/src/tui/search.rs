//! Vim-style `/` search on the workspace tree.
//!
//! Matches include folded rows. Focusing a match unfolds its ancestors
//! so the row is visible. Hidden ignored repos are not in the tree unless
//! shown (`.` / `-a`).

use std::collections::HashSet;

use super::tree::{flatten, TreeNode};

/// Case-insensitive substring matches on `label`. Empty query → no hits.
pub fn match_indices(labels: &[&str], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    labels
        .iter()
        .enumerate()
        .filter(|(_, label)| label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// Next/prev match id with wrap. Empty `ids` → `None`.
pub fn step_match_id(ids: &[String], current_id: Option<&str>, dir: i32) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let pos = current_id.and_then(|id| ids.iter().position(|x| x == id));
    let Some(pos) = pos else {
        return if dir < 0 {
            ids.last().cloned()
        } else {
            ids.first().cloned()
        };
    };
    let len = ids.len() as i32;
    let next = (pos as i32 + dir).rem_euclid(len) as usize;
    ids.get(next).cloned()
}

/// Stable ids whose labels match `query`, in tree order (including folded).
pub fn collect_match_ids(tree: &TreeNode, query: &str) -> Vec<String> {
    let all = flatten(tree, &HashSet::new());
    let labels: Vec<&str> = all.iter().map(|r| r.label.as_str()).collect();
    match_indices(&labels, query)
        .into_iter()
        .map(|i| all[i].id.clone())
        .collect()
}

/// Path from the root to `target` (inclusive). Empty when missing.
pub fn path_to(tree: &TreeNode, target: &str) -> Vec<String> {
    let mut path = Vec::new();
    if find_path(tree, target, &mut path) {
        path
    } else {
        Vec::new()
    }
}

fn find_path(node: &TreeNode, target: &str, path: &mut Vec<String>) -> bool {
    path.push(node.id.clone());
    if node.id == target {
        return true;
    }
    for child in &node.children {
        if find_path(child, target, path) {
            return true;
        }
    }
    path.pop();
    false
}

/// Unfold every ancestor of `focus_id` so the row can paint.
pub fn unfold_ancestors(tree: &TreeNode, folds: &HashSet<String>, focus_id: &str) -> HashSet<String> {
    let mut next = folds.clone();
    for id in path_to(tree, focus_id) {
        next.remove(&id);
    }
    next
}

/// Focus a match. `dir` is `0` (first), `1` (next), or `-1` (previous).
/// Unfolds ancestors of the chosen match only.
pub fn focus_tree_search(
    tree: &TreeNode,
    folds: &HashSet<String>,
    query: &str,
    current_id: Option<&str>,
    dir: i32,
) -> (HashSet<String>, Option<String>) {
    let ids = collect_match_ids(tree, query);
    let focus_id = if dir == 0 {
        ids.first().cloned()
    } else {
        step_match_id(&ids, current_id, dir)
    };
    let Some(focus_id) = focus_id else {
        return (folds.clone(), None);
    };
    (unfold_ancestors(tree, folds, &focus_id), Some(focus_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::tree::{build_tree, default_folds, visible_for_tree, NodeKind};

    fn repo(name: &str, dirty_path: Option<&str>) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            has_unstaged: dirty_path.is_some(),
            has_staged: false,
            has_untracked: false,
            changes: dirty_path
                .map(|path| {
                    vec![FileChange {
                        path: path.into(),
                        staged_status: None,
                        unstaged_status: Some("M".into()),
                        untracked: false,
                        old_path: None,
                    }]
                })
                .unwrap_or_default(),
            checkout_kind: crate::snapshot::CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
        }
    }

    fn tree(show_ignored: bool) -> crate::tui::tree::TreeNode {
        let built = build_workspace_snapshot(
            &[
                repo("app", Some("README.md")),
                repo("lib", None),
                repo("notes", Some("secret.md")),
            ],
            &["notes".into()],
            show_ignored,
            &[],
        );
        build_tree(&visible_for_tree(&built), true)
    }

    #[test]
    fn next_and_prev_wrap() {
        let ids = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(step_match_id(&ids, Some("a"), 1).as_deref(), Some("b"));
        assert_eq!(step_match_id(&ids, Some("c"), 1).as_deref(), Some("a"));
        assert_eq!(step_match_id(&ids, Some("a"), -1).as_deref(), Some("c"));
        assert_eq!(step_match_id(&ids, None, 1).as_deref(), Some("a"));
        assert_eq!(step_match_id(&ids, None, -1).as_deref(), Some("c"));
    }

    #[test]
    fn first_match_unfolds_parent() {
        let tree = tree(false);
        let folds = default_folds(&tree);
        let (next_folds, id) = focus_tree_search(&tree, &folds, "README", None, 0);
        assert_eq!(id.as_deref(), Some("file:app:README.md"));
        let rows = flatten(&tree, &next_folds);
        assert!(rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn n_then_n_prev_walks_matches_and_unfolds() {
        let tree = tree(false);
        let mut folds = HashSet::new();
        folds.insert("repo:app".into());
        folds.insert("group:no-updates".into());
        let (folds, first) = focus_tree_search(&tree, &folds, "main", None, 0);
        assert!(first.is_some());
        let (folds, second) = focus_tree_search(&tree, &folds, "main", first.as_deref(), 1);
        assert_ne!(second, first);
        let (folds, prev) = focus_tree_search(&tree, &folds, "main", second.as_deref(), -1);
        assert_eq!(prev, first);
        if let Some(id) = prev {
            let rows = flatten(&tree, &folds);
            assert!(rows.iter().any(|r| r.id == id));
        }
    }

    #[test]
    fn hidden_ignored_is_not_a_search_hit() {
        let hidden = tree(false);
        assert!(collect_match_ids(&hidden, "secret").is_empty());
        assert!(collect_match_ids(&hidden, "notes").is_empty());
        let shown = tree(true);
        assert_eq!(
            collect_match_ids(&shown, "secret"),
            vec!["file:notes:secret.md".to_string()]
        );
        assert!(shown
            .children
            .iter()
            .any(|c| c.kind == NodeKind::Repo && c.label.contains("notes"))
            || flatten(&shown, &HashSet::new())
                .iter()
                .any(|r| r.label.contains("notes")));
    }
}

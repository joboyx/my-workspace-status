//! Workspace tree from the same snapshot used by --plain / --json.

use std::collections::{BTreeMap, HashSet};

use crate::helpers::{is_attention_sync_note, is_default_branch};
use crate::snapshot::{CheckoutKind, FileChange, WorkspaceRepoSnapshot, WorkspaceSnapshot};

/// Structural node kind. Matches the Ink tree vocabulary used daily.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Workspace,
    Repo,
    Checkout,
    Group,
    File,
}

/// One node in the workspace tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    /// Checkout path used for fetch / pull / default / graph.
    pub repo: Option<String>,
    pub primary_repo: Option<String>,
    pub ignored: bool,
    pub file: Option<FileChange>,
    pub children: Vec<TreeNode>,
}

/// One painted row after fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleRow {
    pub id: String,
    pub depth: usize,
    pub kind: NodeKind,
    pub label: String,
    pub repo: Option<String>,
    pub primary_repo: Option<String>,
    pub ignored: bool,
    pub file: Option<FileChange>,
    pub foldable: bool,
    pub folded: bool,
}

/// Build the workspace tree from a snapshot (ignored repos may still be present).
pub fn build_tree(snapshot: &WorkspaceSnapshot) -> TreeNode {
    let mut families: BTreeMap<String, Vec<&WorkspaceRepoSnapshot>> = BTreeMap::new();
    for repo in &snapshot.repos {
        let key = repo.primary_repo.clone().unwrap_or_else(|| repo.repo.clone());
        families.entry(key).or_default().push(repo);
    }

    let mut attention = Vec::new();
    let mut idle = Vec::new();
    for (primary, members) in families {
        if family_needs_attention(&members) {
            attention.push(family_node(&primary, members));
        } else {
            idle.push(family_node(&primary, members));
        }
    }

    let mut children = attention;
    if !idle.is_empty() {
        children.push(TreeNode {
            id: "group:no-updates".into(),
            kind: NodeKind::Group,
            label: format!("No updates ({})", idle.len()),
            repo: None,
            primary_repo: None,
            ignored: false,
            file: None,
            children: idle,
        });
    }

    let change_count = snapshot
        .repos
        .iter()
        .filter(|r| r.has_unstaged || r.has_staged || r.has_untracked)
        .count();
    TreeNode {
        id: "workspace".into(),
        kind: NodeKind::Workspace,
        label: if snapshot.repos.is_empty() {
            "workspace  no repos".into()
        } else {
            format!("workspace  {change_count} dirty")
        },
        repo: None,
        primary_repo: None,
        ignored: false,
        file: None,
        children,
    }
}

fn family_needs_attention(members: &[&WorkspaceRepoSnapshot]) -> bool {
    members.iter().any(|repo| checkout_needs_attention(repo))
}

fn checkout_needs_attention(repo: &WorkspaceRepoSnapshot) -> bool {
    if repo.has_unstaged || repo.has_staged || repo.has_untracked {
        return true;
    }
    if !is_default_branch(&repo.branch, repo.default_branch_override.as_deref()) {
        return true;
    }
    matches!(
        repo.sync_status,
        crate::snapshot::SyncStatus::Behind
            | crate::snapshot::SyncStatus::Ahead
            | crate::snapshot::SyncStatus::Diverged
    ) || is_attention_sync_note(&repo.sync_note)
}

fn family_node(primary: &str, mut members: Vec<&WorkspaceRepoSnapshot>) -> TreeNode {
    members.sort_by(|a, b| {
        let a_linked = i32::from(a.checkout_kind == CheckoutKind::Linked);
        let b_linked = i32::from(b.checkout_kind == CheckoutKind::Linked);
        a_linked.cmp(&b_linked).then_with(|| a.repo.cmp(&b.repo))
    });
    let has_linked = members
        .iter()
        .any(|m| m.checkout_kind == CheckoutKind::Linked);
    if !has_linked {
        let repo = members[0];
        return repo_or_checkout(repo, NodeKind::Repo);
    }
    let children = members
        .iter()
        .map(|m| repo_or_checkout(m, NodeKind::Checkout))
        .collect();
    let ignored = members.iter().all(|m| m.ignored);
    TreeNode {
        id: format!("repo:{primary}"),
        kind: NodeKind::Repo,
        label: format!("{primary}  {} wt", members.len()),
        repo: Some(primary.to_string()),
        primary_repo: None,
        ignored,
        file: None,
        children,
    }
}

fn repo_or_checkout(repo: &WorkspaceRepoSnapshot, kind: NodeKind) -> TreeNode {
    let files = repo
        .changes
        .iter()
        .map(|change| TreeNode {
            id: format!("file:{}:{}", repo.repo, change.path),
            kind: NodeKind::File,
            label: file_label(change),
            repo: Some(repo.repo.clone()),
            primary_repo: repo.primary_repo.clone(),
            ignored: repo.ignored,
            file: Some(change.clone()),
            children: Vec::new(),
        })
        .collect();
    let prefix = if repo.checkout_kind == CheckoutKind::Linked {
        "wt "
    } else {
        ""
    };
    let dirty = if repo.has_unstaged || repo.has_staged || repo.has_untracked {
        format!("  {} files", repo.changes.len())
    } else {
        String::new()
    };
    let ignore_mark = if repo.ignored { "  [ignored]" } else { "" };
    TreeNode {
        id: match kind {
            NodeKind::Checkout => format!("checkout:{}", repo.repo),
            _ => format!("repo:{}", repo.repo),
        },
        kind,
        label: format!(
            "{prefix}{}  {}{dirty}{ignore_mark}",
            repo.repo, repo.branch
        ),
        repo: Some(repo.repo.clone()),
        primary_repo: repo.primary_repo.clone(),
        ignored: repo.ignored,
        file: None,
        children: files,
    }
}

fn file_label(change: &FileChange) -> String {
    let mark = if change.untracked {
        "?"
    } else if change.staged_status.is_some() && change.unstaged_status.is_some() {
        "M+"
    } else if let Some(st) = change.staged_status.as_deref() {
        st
    } else if let Some(st) = change.unstaged_status.as_deref() {
        st
    } else {
        "M"
    };
    format!("{mark} {}", change.path)
}

/// Default folds: the No updates group starts closed.
pub fn default_folds(tree: &TreeNode) -> HashSet<String> {
    let mut folds = HashSet::new();
    if tree
        .children
        .iter()
        .any(|c| c.id == "group:no-updates")
    {
        folds.insert("group:no-updates".into());
    }
    folds
}

/// Depth-first flatten, honoring `folds`.
pub fn flatten(tree: &TreeNode, folds: &HashSet<String>) -> Vec<VisibleRow> {
    let mut out = Vec::new();
    walk(tree, 0, folds, &mut out);
    out
}

fn walk(node: &TreeNode, depth: usize, folds: &HashSet<String>, out: &mut Vec<VisibleRow>) {
    let foldable = !node.children.is_empty();
    let folded = foldable && folds.contains(&node.id);
    out.push(VisibleRow {
        id: node.id.clone(),
        depth,
        kind: node.kind,
        label: node.label.clone(),
        repo: node.repo.clone(),
        primary_repo: node.primary_repo.clone(),
        ignored: node.ignored,
        file: node.file.clone(),
        foldable,
        folded,
    });
    if folded {
        return;
    }
    for child in &node.children {
        walk(child, depth + 1, folds, out);
    }
}

/// Visible snapshot used for the tree: hidden ignored repos stay out.
pub fn visible_for_tree(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot {
    crate::snapshot::visible_workspace_snapshot(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };

    fn repo(name: &str, ignored_dirty: bool, linked: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            has_unstaged: ignored_dirty,
            has_staged: false,
            has_untracked: false,
            changes: if ignored_dirty {
                vec![FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                }]
            } else {
                vec![]
            },
            checkout_kind: if linked {
                CheckoutKind::Linked
            } else {
                CheckoutKind::Primary
            },
            primary_repo: if linked {
                Some("app".into())
            } else {
                None
            },
            merged_into_default: None,
            default_branch_override: None,
        }
    }

    #[test]
    fn hidden_ignored_omitted_from_visible_tree() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("notes", true, false)],
            &["notes".into()],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built));
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().any(|r| r.label.contains("app")));
        assert!(rows.iter().all(|r| !r.label.contains("notes")));
    }

    #[test]
    fn show_ignored_includes_notes() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("notes", true, false)],
            &["notes".into()],
            true,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built));
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().any(|r| r.label.contains("notes")));
    }

    #[test]
    fn no_updates_group_starts_folded() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("lib", false, false)],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built));
        let folds = default_folds(&tree);
        assert!(folds.contains("group:no-updates"));
        let rows = flatten(&tree, &folds);
        assert!(rows.iter().any(|r| r.id == "group:no-updates" && r.folded));
        assert!(rows.iter().all(|r| !r.label.contains("lib  main") || r.id == "group:no-updates"));
    }
}

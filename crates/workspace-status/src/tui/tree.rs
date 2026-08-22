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
    Dir,
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
/// `tree_mode` true is a directory trie (Ink default). False is a flat path list.
pub fn build_tree(snapshot: &WorkspaceSnapshot, tree_mode: bool) -> TreeNode {
    let mut families: BTreeMap<String, Vec<&WorkspaceRepoSnapshot>> = BTreeMap::new();
    for repo in &snapshot.repos {
        let key = repo.primary_repo.clone().unwrap_or_else(|| repo.repo.clone());
        families.entry(key).or_default().push(repo);
    }

    let mut attention = Vec::new();
    let mut idle = Vec::new();
    for (primary, members) in families {
        if family_needs_attention(&members) {
            attention.push(family_node(&primary, members, tree_mode));
        } else {
            idle.push(family_node(&primary, members, tree_mode));
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

fn family_node(
    primary: &str,
    mut members: Vec<&WorkspaceRepoSnapshot>,
    tree_mode: bool,
) -> TreeNode {
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
        return repo_or_checkout(repo, NodeKind::Repo, tree_mode);
    }
    let children = members
        .iter()
        .map(|m| repo_or_checkout(m, NodeKind::Checkout, tree_mode))
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

fn repo_or_checkout(repo: &WorkspaceRepoSnapshot, kind: NodeKind, tree_mode: bool) -> TreeNode {
    let files = materialize_change_forest(repo, tree_mode);
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

fn file_label(change: &FileChange, tree_mode: bool) -> String {
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
    let name = if tree_mode {
        change
            .path
            .rsplit('/')
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or(change.path.as_str())
    } else {
        change.path.as_str()
    };
    format!("{mark} {name}")
}

#[derive(Default)]
struct MutableDir {
    dirs: BTreeMap<String, MutableDir>,
    files: Vec<FileChange>,
}

fn add_change(root: &mut MutableDir, change: &FileChange) {
    let mut parts: Vec<&str> = change.path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.pop().is_none() {
        return;
    }
    let mut node = root;
    for dir in parts {
        node = node
            .dirs
            .entry(dir.to_string())
            .or_insert_with(MutableDir::default);
    }
    node.files.push(change.clone());
}

fn collapse_dir(name: String, mut node: MutableDir) -> (String, MutableDir) {
    let mut collapsed_name = name;
    while node.files.is_empty() && node.dirs.len() == 1 {
        let (child_name, child_node) = node.dirs.into_iter().next().expect("one child dir");
        collapsed_name = format!("{collapsed_name}/{child_name}");
        node = child_node;
    }
    (collapsed_name, node)
}

fn make_file_node(
    repo: &WorkspaceRepoSnapshot,
    change: &FileChange,
    tree_mode: bool,
) -> TreeNode {
    TreeNode {
        id: format!("file:{}:{}", repo.repo, change.path),
        kind: NodeKind::File,
        label: file_label(change, tree_mode),
        repo: Some(repo.repo.clone()),
        primary_repo: repo.primary_repo.clone(),
        ignored: repo.ignored,
        file: Some(change.clone()),
        children: Vec::new(),
    }
}

fn materialize_dir(
    repo: &WorkspaceRepoSnapshot,
    dir_path: &str,
    node: MutableDir,
    tree_mode: bool,
) -> Vec<TreeNode> {
    let mut dir_entries: Vec<(String, MutableDir)> = node
        .dirs
        .into_iter()
        .map(|(name, child)| collapse_dir(name, child))
        .collect();
    dir_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut file_entries = node.files;
    file_entries.sort_by(|a, b| a.path.cmp(&b.path));

    let mut children = Vec::new();
    for (name, child) in dir_entries {
        let full_path = if dir_path.is_empty() {
            name.clone()
        } else {
            format!("{dir_path}/{name}")
        };
        children.push(TreeNode {
            id: format!("dir:{}:{full_path}", repo.repo),
            kind: NodeKind::Dir,
            label: name,
            repo: Some(repo.repo.clone()),
            primary_repo: repo.primary_repo.clone(),
            ignored: repo.ignored,
            file: None,
            children: materialize_dir(repo, &full_path, child, tree_mode),
        });
    }
    for change in file_entries {
        children.push(make_file_node(repo, &change, tree_mode));
    }
    children
}

/// File / dir forest under a checkout. Matches Ink `materializeChangeForest`.
fn materialize_change_forest(repo: &WorkspaceRepoSnapshot, tree_mode: bool) -> Vec<TreeNode> {
    if !tree_mode {
        return repo
            .changes
            .iter()
            .map(|change| make_file_node(repo, change, false))
            .collect();
    }
    let mut root = MutableDir::default();
    for change in &repo.changes {
        add_change(&mut root, change);
    }
    materialize_dir(repo, "", root, true)
}

/// True when `path` is the dir itself or a child of it.
pub fn path_under_dir(path: &str, dir: &str) -> bool {
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// Dir path from a `dir:{{repo}}:{{fullPath}}` row id.
pub fn dir_path_from_id(id: &str, repo: &str) -> Option<String> {
    let prefix = format!("dir:{repo}:");
    id.strip_prefix(&prefix).map(str::to_string)
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

/// Find a node by id.
pub fn find_node<'a>(node: &'a TreeNode, id: &str) -> Option<&'a TreeNode> {
    if node.id == id {
        return Some(node);
    }
    for child in &node.children {
        if let Some(hit) = find_node(child, id) {
            return Some(hit);
        }
    }
    None
}

fn walk_foldable(node: &TreeNode, out: &mut Vec<String>) {
    match node.kind {
        NodeKind::Workspace | NodeKind::Repo | NodeKind::Checkout | NodeKind::Group | NodeKind::Dir => {
            out.push(node.id.clone());
            for child in &node.children {
                walk_foldable(child, out);
            }
        }
        NodeKind::File => {}
    }
}

/// Foldable ids for `focus_id` and every foldable descendant under it.
/// Empty when the id is missing or names a file.
pub fn collect_foldable_subtree_ids(tree: &TreeNode, focus_id: &str) -> Vec<String> {
    let Some(found) = find_node(tree, focus_id) else {
        return Vec::new();
    };
    if found.kind == NodeKind::File {
        return Vec::new();
    }
    let mut ids = Vec::new();
    walk_foldable(found, &mut ids);
    ids
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
        let tree = build_tree(&visible_for_tree(&built), true);
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
        let tree = build_tree(&visible_for_tree(&built), true);
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
        let tree = build_tree(&visible_for_tree(&built), true);
        let folds = default_folds(&tree);
        assert!(folds.contains("group:no-updates"));
        let rows = flatten(&tree, &folds);
        assert!(rows.iter().any(|r| r.id == "group:no-updates" && r.folded));
        assert!(rows.iter().all(|r| !r.label.contains("lib  main") || r.id == "group:no-updates"));
    }

    fn dirty_repo(name: &str, paths: &[&str]) -> RepoSnapshot {
        let mut snap = repo(name, true, false);
        snap.has_untracked = paths.iter().any(|p| *p != "README.md" && *p != "src/lib.rs");
        snap.changes = paths
            .iter()
            .map(|path| FileChange {
                path: (*path).into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            })
            .collect();
        snap
    }

    #[test]
    fn tree_mode_inserts_dir_and_basename_file() {
        let built = build_workspace_snapshot(
            &[dirty_repo("app", &["src/lib.rs", "README.md"])],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true);
        let rows = flatten(&tree, &HashSet::new());
        let dir = rows
            .iter()
            .find(|r| r.id == "dir:app:src")
            .expect("dir:app:src");
        assert_eq!(dir.kind, NodeKind::Dir);
        assert!(dir.foldable);
        assert_eq!(dir.label, "src");
        let lib = rows
            .iter()
            .find(|r| r.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert_eq!(lib.kind, NodeKind::File);
        assert!(lib.label.contains("lib.rs"));
        assert!(!lib.label.contains("src/lib.rs"));
        let readme = rows
            .iter()
            .find(|r| r.id == "file:app:README.md")
            .expect("README.md");
        assert!(readme.label.contains("README.md"));
        let dir_idx = rows.iter().position(|r| r.id == "dir:app:src").unwrap();
        let readme_idx = rows.iter().position(|r| r.id == "file:app:README.md").unwrap();
        let repo_idx = rows.iter().position(|r| r.id == "repo:app").unwrap();
        assert!(dir_idx < readme_idx);
        assert_eq!(rows[dir_idx + 1].id, "file:app:src/lib.rs");
        assert!(repo_idx < dir_idx);
    }

    #[test]
    fn flat_mode_is_full_paths_without_dir_rows() {
        let built = build_workspace_snapshot(
            &[dirty_repo("app", &["src/lib.rs", "README.md"])],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), false);
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().all(|r| r.kind != NodeKind::Dir));
        let lib = rows
            .iter()
            .find(|r| r.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("src/lib.rs"));
        assert!(rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn collapse_single_child_dirs() {
        let built = build_workspace_snapshot(
            &[dirty_repo("app", &["src/foo/bar.rs"])],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true);
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().any(|r| r.id == "dir:app:src/foo" && r.label == "src/foo"));
        assert!(rows.iter().all(|r| r.id != "dir:app:src"));
        let file = rows
            .iter()
            .find(|r| r.id == "file:app:src/foo/bar.rs")
            .expect("bar.rs");
        assert!(file.label.contains("bar.rs"));
        assert!(!file.label.contains("src/foo/bar.rs"));
    }
}

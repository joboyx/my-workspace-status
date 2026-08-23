//! Commit-file directory trie. Same collapse as the workspace change forest.
//!
//! File chrome (icon + trailing status badge) is `tree::file_change_segments`.

use std::collections::{BTreeMap, HashSet};

use super::drill::CommitFile;
use super::tree::{
    dir_name_segments, file_change_from_name_status, file_change_segments, segments_search_label,
    NodeSegments, SegRole, TextSeg,
};

/// Kind of one painted commit-file row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitFileRowKind {
    Dir,
    File,
}

/// One painted commit-file row after fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFileRow {
    pub id: String,
    pub depth: usize,
    pub kind: CommitFileRowKind,
    pub label: String,
    /// Right-aligned status badge (files) or empty (dirs).
    pub trailing: String,
    pub segments: Vec<TextSeg>,
    pub trailing_segs: Vec<TextSeg>,
    pub path: String,
    pub foldable: bool,
    pub folded: bool,
    pub file: Option<CommitFile>,
}

impl CommitFileRow {
    pub fn is_dir(&self) -> bool {
        self.kind == CommitFileRowKind::Dir
    }

    pub fn is_file(&self) -> bool {
        self.kind == CommitFileRowKind::File
    }
}

#[derive(Default)]
struct MutableDir {
    dirs: BTreeMap<String, MutableDir>,
    files: Vec<CommitFile>,
}

/// Dir / file forest for a commit, stash, or worktree file list.
pub fn materialize_commit_file_forest(
    files: &[CommitFile],
    tree_mode: bool,
) -> Vec<CommitFileNode> {
    if !tree_mode {
        return files
            .iter()
            .map(|file| make_file_node(file, false))
            .collect();
    }
    let mut root = MutableDir::default();
    for file in files {
        add_file(&mut root, file);
    }
    materialize_dir("", root)
}

/// Flatten a commit-file forest, honoring `folds`.
///
/// `ascii` selects the same glyph fallback as the workspace tree.
pub fn flatten_commit_files(
    files: &[CommitFile],
    tree_mode: bool,
    folds: &HashSet<String>,
    ascii: bool,
) -> Vec<CommitFileRow> {
    let nodes = materialize_commit_file_forest(files, tree_mode);
    let mut out = Vec::new();
    for node in &nodes {
        walk(node, 0, folds, tree_mode, ascii, &mut out);
    }
    out
}

/// Dir or file node in the commit-file forest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFileNode {
    pub id: String,
    pub kind: CommitFileRowKind,
    pub label: String,
    pub path: String,
    pub file: Option<CommitFile>,
    pub children: Vec<CommitFileNode>,
}

fn add_file(root: &mut MutableDir, file: &CommitFile) {
    let mut parts: Vec<&str> = file
        .path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
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
    node.files.push(file.clone());
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

fn file_node_label(file: &CommitFile, tree_mode: bool) -> String {
    if tree_mode {
        file.path
            .rsplit('/')
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or(file.path.as_str())
            .to_string()
    } else {
        file.path.clone()
    }
}

fn make_file_node(file: &CommitFile, tree_mode: bool) -> CommitFileNode {
    CommitFileNode {
        id: format!("file:{}", file.path),
        kind: CommitFileRowKind::File,
        label: file_node_label(file, tree_mode),
        path: file.path.clone(),
        file: Some(file.clone()),
        children: Vec::new(),
    }
}

fn materialize_dir(dir_path: &str, node: MutableDir) -> Vec<CommitFileNode> {
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
        children.push(CommitFileNode {
            id: format!("dir:{full_path}"),
            kind: CommitFileRowKind::Dir,
            label: name,
            path: full_path.clone(),
            file: None,
            children: materialize_dir(&full_path, child),
        });
    }
    for file in file_entries {
        children.push(make_file_node(&file, true));
    }
    children
}

fn commit_file_segments(node: &CommitFileNode, tree_mode: bool, ascii: bool) -> NodeSegments {
    match node.kind {
        CommitFileRowKind::Dir => dir_name_segments(&node.label, ascii),
        CommitFileRowKind::File => match node.file.as_ref() {
            Some(file) => {
                let change = file_change_from_name_status(
                    &file.status,
                    file.path.clone(),
                    file.old_path.clone(),
                );
                file_change_segments(&change, tree_mode, ascii)
            }
            None => NodeSegments {
                segments: vec![TextSeg {
                    text: node.label.clone(),
                    role: SegRole::File,
                    hex: None,
                    bold: false,
                    dim: false,
                }],
                trailing: Vec::new(),
            },
        },
    }
}

fn walk(
    node: &CommitFileNode,
    depth: usize,
    folds: &HashSet<String>,
    tree_mode: bool,
    ascii: bool,
    out: &mut Vec<CommitFileRow>,
) {
    let foldable = !node.children.is_empty();
    let folded = foldable && folds.contains(&node.id);
    let segs = commit_file_segments(node, tree_mode, ascii);
    let (label, trailing) = segments_search_label(&segs);
    out.push(CommitFileRow {
        id: node.id.clone(),
        depth,
        kind: node.kind,
        label,
        trailing,
        segments: segs.segments,
        trailing_segs: segs.trailing,
        path: node.path.clone(),
        foldable,
        folded,
        file: node.file.clone(),
    });
    if folded {
        return;
    }
    for child in &node.children {
        walk(child, depth + 1, folds, tree_mode, ascii, out);
    }
}

fn find_commit_node<'a>(node: &'a CommitFileNode, id: &str) -> Option<&'a CommitFileNode> {
    if node.id == id {
        return Some(node);
    }
    for child in &node.children {
        if let Some(hit) = find_commit_node(child, id) {
            return Some(hit);
        }
    }
    None
}

fn walk_commit_foldable(node: &CommitFileNode, out: &mut Vec<String>) {
    if node.kind == CommitFileRowKind::Dir || !node.children.is_empty() {
        out.push(node.id.clone());
        for child in &node.children {
            walk_commit_foldable(child, out);
        }
    }
}

/// Foldable ids for `focus_id` and foldable descendants in the commit-file forest.
pub fn collect_foldable_subtree_ids(
    files: &[CommitFile],
    tree_mode: bool,
    focus_id: &str,
) -> Vec<String> {
    let nodes = materialize_commit_file_forest(files, tree_mode);
    for node in &nodes {
        if let Some(found) = find_commit_node(node, focus_id) {
            if found.kind == CommitFileRowKind::File {
                return Vec::new();
            }
            let mut ids = Vec::new();
            walk_commit_foldable(found, &mut ids);
            return ids;
        }
    }
    Vec::new()
}

/// Dir ids that must unfold so `file_path` is visible.
pub fn ancestor_dir_ids(file_path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = file_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 2 {
        return Vec::new();
    }
    parts.pop();
    let mut ids = Vec::new();
    let mut acc = String::new();
    for part in parts {
        if acc.is_empty() {
            acc = part.to_string();
        } else {
            acc = format!("{acc}/{part}");
        }
        ids.push(format!("dir:{acc}"));
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(status: &str, path: &str) -> CommitFile {
        CommitFile {
            status: status.into(),
            path: path.into(),
            old_path: None,
        }
    }

    #[test]
    fn tree_mode_inserts_dir_and_basename() {
        let files = vec![file("A", "src/lib.rs"), file("M", "README.md")];
        let rows = flatten_commit_files(&files, true, &HashSet::new(), true);
        let dir = rows.iter().find(|r| r.id == "dir:src").expect("dir");
        assert_eq!(dir.kind, CommitFileRowKind::Dir);
        assert!(dir.foldable);
        assert!(dir.label.contains("src"));
        let lib = rows
            .iter()
            .find(|r| r.id == "file:src/lib.rs")
            .expect("lib.rs");
        assert_eq!(lib.kind, CommitFileRowKind::File);
        assert!(lib.label.contains("lib.rs"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert_eq!(lib.trailing.trim(), "A");
        assert!(!lib.label.contains("A  lib"));
        let readme = rows
            .iter()
            .find(|r| r.id == "file:README.md")
            .expect("README");
        assert_eq!(readme.trailing.trim(), "M");
        let dir_idx = rows.iter().position(|r| r.id == "dir:src").unwrap();
        assert_eq!(rows[dir_idx + 1].id, "file:src/lib.rs");
    }

    #[test]
    fn flat_mode_is_full_paths_without_dirs() {
        let files = vec![file("A", "src/lib.rs"), file("M", "README.md")];
        let rows = flatten_commit_files(&files, false, &HashSet::new(), true);
        assert!(rows.iter().all(|r| r.kind != CommitFileRowKind::Dir));
        let lib = rows
            .iter()
            .find(|r| r.id == "file:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("lib.rs"));
        assert!(lib.label.contains("src"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert_eq!(lib.trailing.trim(), "A");
    }

    #[test]
    fn collapse_single_child_dirs() {
        let files = vec![file("A", "src/foo/bar.rs")];
        let rows = flatten_commit_files(&files, true, &HashSet::new(), true);
        assert!(rows
            .iter()
            .any(|r| r.id == "dir:src/foo" && r.label.contains("src/foo")));
        assert!(rows.iter().all(|r| r.id != "dir:src"));
        let file_row = rows
            .iter()
            .find(|r| r.id == "file:src/foo/bar.rs")
            .expect("bar.rs");
        assert!(file_row.label.contains("bar.rs"));
        assert!(!file_row.label.contains("src/foo/bar.rs"));
    }

    #[test]
    fn fold_hides_children() {
        let files = vec![file("A", "src/lib.rs")];
        let mut folds = HashSet::new();
        folds.insert("dir:src".into());
        let rows = flatten_commit_files(&files, true, &folds, true);
        assert!(rows.iter().any(|r| r.id == "dir:src" && r.folded));
        assert!(rows.iter().all(|r| r.id != "file:src/lib.rs"));
    }
}

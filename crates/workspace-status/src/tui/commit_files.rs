//! Commit-file directory trie. Same collapse as the workspace change forest.

use std::collections::{BTreeMap, HashSet};

use super::drill::CommitFile;

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
pub fn materialize_commit_file_forest(files: &[CommitFile], tree_mode: bool) -> Vec<CommitFileNode> {
    if !tree_mode {
        return files.iter().map(|file| make_file_node(file, false)).collect();
    }
    let mut root = MutableDir::default();
    for file in files {
        add_file(&mut root, file);
    }
    materialize_dir("", root)
}

/// Flatten a commit-file forest, honoring `folds`.
pub fn flatten_commit_files(
    files: &[CommitFile],
    tree_mode: bool,
    folds: &HashSet<String>,
) -> Vec<CommitFileRow> {
    let nodes = materialize_commit_file_forest(files, tree_mode);
    let mut out = Vec::new();
    for node in &nodes {
        walk(node, 0, folds, &mut out);
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
    let mut parts: Vec<&str> = file.path.split('/').filter(|part| !part.is_empty()).collect();
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

fn file_label(file: &CommitFile, tree_mode: bool) -> String {
    let name = if tree_mode {
        file.path
            .rsplit('/')
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or(file.path.as_str())
    } else {
        file.path.as_str()
    };
    format!("{}  {name}", file.status)
}

fn make_file_node(file: &CommitFile, tree_mode: bool) -> CommitFileNode {
    CommitFileNode {
        id: format!("file:{}", file.path),
        kind: CommitFileRowKind::File,
        label: file_label(file, tree_mode),
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

fn walk(node: &CommitFileNode, depth: usize, folds: &HashSet<String>, out: &mut Vec<CommitFileRow>) {
    let foldable = !node.children.is_empty();
    let folded = foldable && folds.contains(&node.id);
    out.push(CommitFileRow {
        id: node.id.clone(),
        depth,
        kind: node.kind,
        label: node.label.clone(),
        path: node.path.clone(),
        foldable,
        folded,
        file: node.file.clone(),
    });
    if folded {
        return;
    }
    for child in &node.children {
        walk(child, depth + 1, folds, out);
    }
}

/// Dir ids that must unfold so `file_path` is visible.
pub fn ancestor_dir_ids(file_path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = file_path.split('/').filter(|part| !part.is_empty()).collect();
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
        let rows = flatten_commit_files(&files, true, &HashSet::new());
        let dir = rows.iter().find(|r| r.id == "dir:src").expect("dir");
        assert_eq!(dir.kind, CommitFileRowKind::Dir);
        assert!(dir.foldable);
        assert_eq!(dir.label, "src");
        let lib = rows
            .iter()
            .find(|r| r.id == "file:src/lib.rs")
            .expect("lib.rs");
        assert_eq!(lib.kind, CommitFileRowKind::File);
        assert!(lib.label.contains("lib.rs"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert!(rows.iter().any(|r| r.id == "file:README.md"));
        let dir_idx = rows.iter().position(|r| r.id == "dir:src").unwrap();
        assert_eq!(rows[dir_idx + 1].id, "file:src/lib.rs");
    }

    #[test]
    fn flat_mode_is_full_paths_without_dirs() {
        let files = vec![file("A", "src/lib.rs"), file("M", "README.md")];
        let rows = flatten_commit_files(&files, false, &HashSet::new());
        assert!(rows.iter().all(|r| r.kind != CommitFileRowKind::Dir));
        let lib = rows
            .iter()
            .find(|r| r.id == "file:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("src/lib.rs"));
    }

    #[test]
    fn collapse_single_child_dirs() {
        let files = vec![file("A", "src/foo/bar.rs")];
        let rows = flatten_commit_files(&files, true, &HashSet::new());
        assert!(rows
            .iter()
            .any(|r| r.id == "dir:src/foo" && r.label == "src/foo"));
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
        let rows = flatten_commit_files(&files, true, &folds);
        assert!(rows.iter().any(|r| r.id == "dir:src" && r.folded));
        assert!(rows.iter().all(|r| r.id != "file:src/lib.rs"));
    }
}

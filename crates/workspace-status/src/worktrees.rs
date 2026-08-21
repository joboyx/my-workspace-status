//! Worktree porcelain parse, under-cwd mapping, merge classification.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::helpers::DETACHED_HEAD_BRANCH;

#[derive(Debug, Clone)]
pub struct GitWorktreeListEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathIdentity {
    pub dev: u64,
    pub ino: u64,
}

fn realpath(abs: &Path) -> PathBuf {
    fs::canonicalize(abs).unwrap_or_else(|_| abs.to_path_buf())
}

fn identity(abs: &Path) -> Option<PathIdentity> {
    fs::metadata(abs).ok().map(|st| PathIdentity {
        dev: st.dev(),
        ino: st.ino(),
    })
}

fn same_identity(a: Option<PathIdentity>, b: Option<PathIdentity>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x == y)
}

fn under_dir(abs: &Path, dir: &Path) -> bool {
    abs == dir || abs.starts_with(dir)
}

fn posix_rel(from: &Path, to: &Path) -> String {
    pathdiff_rel(from, to)
}

fn pathdiff_rel(from: &Path, to: &Path) -> String {
    match to.strip_prefix(from) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => to.to_string_lossy().replace('\\', "/"),
    }
}

fn join_workspace_rel(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("/")
}

fn remap_via_primary_identity(
    abs: &Path,
    cwd: &Path,
    primary: &Path,
    primary_id: PathIdentity,
) -> Option<(PathBuf, String)> {
    let primary_rel = posix_rel(cwd, primary);
    let mut suffix: Vec<String> = Vec::new();
    let mut cur = abs.to_path_buf();
    loop {
        if same_identity(identity(&cur), Some(primary_id)) || cur == primary {
            suffix.reverse();
            let extra = suffix.join("/");
            let rel_path = join_workspace_rel(&[primary_rel.as_str(), extra.as_str()]);
            let abs_path = if rel_path.is_empty() {
                cwd.to_path_buf()
            } else {
                cwd.join(PathBuf::from(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR)))
            };
            return Some((abs_path, rel_path));
        }
        match cur.parent() {
            Some(parent) if parent != cur => {
                if let Some(name) = cur.file_name() {
                    suffix.push(name.to_string_lossy().into_owned());
                }
                cur = parent.to_path_buf();
            }
            _ => break,
        }
    }
    None
}

/// Map a linked worktree absolute path to a workspace-relative path under cwd.
pub fn map_linked_worktree_rel_path(
    entry_abs: &Path,
    cwd_abs: &Path,
    primary_abs: &Path,
) -> Option<(PathBuf, String)> {
    let cwd = realpath(cwd_abs);
    let primary = realpath(primary_abs);
    let abs = realpath(entry_abs);
    let primary_id = identity(&primary);

    if same_identity(identity(&abs), primary_id) || abs == primary {
        return None;
    }
    if !under_dir(&primary, &cwd) {
        return None;
    }
    if under_dir(&abs, &primary) {
        return Some((abs.clone(), posix_rel(&cwd, &abs)));
    }
    if let Some(id) = primary_id {
        if let Some(remapped) = remap_via_primary_identity(&abs, &cwd, &primary, id) {
            return Some(remapped);
        }
    }
    if under_dir(&abs, &cwd) {
        return Some((abs.clone(), posix_rel(&cwd, &abs)));
    }
    None
}

pub fn parse_worktree_list_porcelain(text: &str) -> Vec<GitWorktreeListEntry> {
    let mut entries = Vec::new();
    let mut current: Option<GitWorktreeListEntry> = None;

    let flush = |current: &mut Option<GitWorktreeListEntry>, entries: &mut Vec<GitWorktreeListEntry>| {
        if let Some(entry) = current.take() {
            entries.push(entry);
        }
    };

    for raw in text.split('\n') {
        let line = raw.trim_end();
        if line.is_empty() {
            flush(&mut current, &mut entries);
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(&mut current, &mut entries);
            current = Some(GitWorktreeListEntry {
                path: PathBuf::from(rest),
                head: None,
                branch: None,
                bare: false,
                detached: false,
            });
            continue;
        }
        let Some(cur) = current.as_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("HEAD ") {
            cur.head = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("branch ") {
            cur.branch = Some(
                rest.strip_prefix("refs/heads/")
                    .unwrap_or(rest)
                    .to_string(),
            );
            continue;
        }
        if line == "bare" {
            cur.bare = true;
            continue;
        }
        if line == "detached" {
            cur.detached = true;
        }
    }
    flush(&mut current, &mut entries);
    entries
}

pub fn linked_worktrees_under_cwd(
    entries: &[GitWorktreeListEntry],
    cwd_abs: &Path,
    primary_abs: &Path,
) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for entry in entries {
        if entry.bare {
            continue;
        }
        if let Some(mapped) = map_linked_worktree_rel_path(&entry.path, cwd_abs, primary_abs) {
            out.push(mapped);
        }
    }
    out
}

pub fn classify_merged_into_default(
    branch: &str,
    default_branch: &str,
    is_ancestor_of_default: Option<bool>,
) -> Option<bool> {
    if branch == default_branch {
        return None;
    }
    if branch == DETACHED_HEAD_BRANCH {
        return None;
    }
    is_ancestor_of_default
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_porcelain_branch_and_detached() {
        let text = "\
worktree /tmp/app
HEAD abc
branch refs/heads/main

worktree /tmp/app/.worktrees/feat
HEAD def
detached
";
        let entries = parse_worktree_list_porcelain(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(entries[1].detached);
    }

    #[test]
    fn classify_merge_skips_default_and_detached() {
        assert_eq!(
            classify_merged_into_default("main", "main", Some(true)),
            None
        );
        assert_eq!(
            classify_merged_into_default(DETACHED_HEAD_BRANCH, "main", Some(true)),
            None
        );
        assert_eq!(
            classify_merged_into_default("feature/x", "main", Some(true)),
            Some(true)
        );
    }
}

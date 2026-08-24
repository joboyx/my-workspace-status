//! Worktree porcelain parse, under-cwd mapping, merge classification.

use std::fs;
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

/// Filesystem identity used to match a TUI path to a git-registered worktree.
///
/// Unix uses `(dev, ino)` so bind-mount aliases match. Other platforms have no
/// inodes; identity is canonical path plus size and mtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathIdentity {
    #[cfg(unix)]
    pub dev: u64,
    #[cfg(unix)]
    pub ino: u64,
    #[cfg(not(unix))]
    path: PathBuf,
    #[cfg(not(unix))]
    len: u64,
    #[cfg(not(unix))]
    mtime: Option<std::time::SystemTime>,
}

fn realpath(abs: &Path) -> PathBuf {
    fs::canonicalize(abs).unwrap_or_else(|_| abs.to_path_buf())
}

fn identity(abs: &Path) -> Option<PathIdentity> {
    let st = fs::metadata(abs).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(PathIdentity {
            dev: st.dev(),
            ino: st.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Some(PathIdentity {
            path: abs.to_path_buf(),
            len: st.len(),
            mtime: st.modified().ok(),
        })
    }
}

fn same_identity(a: Option<&PathIdentity>, b: Option<&PathIdentity>) -> bool {
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
        if same_identity(identity(&cur).as_ref(), Some(&primary_id)) || cur == primary {
            suffix.reverse();
            let extra = suffix.join("/");
            let rel_path = join_workspace_rel(&[primary_rel.as_str(), extra.as_str()]);
            let abs_path = if rel_path.is_empty() {
                cwd.to_path_buf()
            } else {
                cwd.join(PathBuf::from(
                    rel_path.replace('/', std::path::MAIN_SEPARATOR_STR),
                ))
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

/// Main checkout: `.git` is a directory. Linked extras use a gitfile.
pub fn is_main_worktree_checkout(dir: &Path) -> bool {
    fs::metadata(dir.join(".git"))
        .map(|st| st.is_dir())
        .unwrap_or(false)
}

/// Linked extra: `.git` is a file (`gitdir: …`), not the main checkout.
pub fn is_linked_worktree_checkout(dir: &Path) -> bool {
    fs::metadata(dir.join(".git"))
        .map(|st| st.is_file())
        .unwrap_or(false)
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

    if same_identity(identity(&abs).as_ref(), primary_id.as_ref()) || abs == primary {
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

    let flush = |current: &mut Option<GitWorktreeListEntry>,
                 entries: &mut Vec<GitWorktreeListEntry>| {
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
            cur.branch = Some(rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string());
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

/// Paths git accepts for `worktree remove` when the TUI path is a bind-mount alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRemoveTarget {
    pub git_cwd: PathBuf,
    pub git_path: PathBuf,
}

fn resolve_abs(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn listed_worktree_path(entries: &[GitWorktreeListEntry], worktree_abs: &Path) -> PathBuf {
    let wt = resolve_abs(worktree_abs);
    for entry in entries {
        if entry.bare {
            continue;
        }
        if resolve_abs(&entry.path) == wt {
            return wt;
        }
    }
    let wt_id = identity(&wt);
    if wt_id.is_none() {
        return wt;
    }
    for entry in entries {
        if entry.bare {
            continue;
        }
        let listed = resolve_abs(&entry.path);
        if same_identity(identity(&listed).as_ref(), wt_id.as_ref()) {
            return listed;
        }
    }
    wt
}

fn registered_primary_abs(abs: &Path, primary_abs: &Path) -> Option<PathBuf> {
    let primary_id = identity(&resolve_abs(primary_abs))?;
    let mut cur = resolve_abs(abs);
    loop {
        if same_identity(identity(&cur).as_ref(), Some(&primary_id)) {
            return Some(cur);
        }
        match cur.parent() {
            Some(parent) if parent != cur => cur = parent.to_path_buf(),
            _ => return None,
        }
    }
}

/// `gitPath` is the porcelain worktree line (inode match on Unix when prefixes
/// differ; path + size + mtime elsewhere).
/// `gitCwd` is the registered primary prefix so gitdir back-pointers match.
pub fn resolve_worktree_remove_target(
    entries: &[GitWorktreeListEntry],
    primary_abs: &Path,
    worktree_abs: &Path,
) -> WorktreeRemoveTarget {
    let git_path = listed_worktree_path(entries, worktree_abs);
    let git_cwd =
        registered_primary_abs(&git_path, primary_abs).unwrap_or_else(|| resolve_abs(primary_abs));
    WorktreeRemoveTarget { git_cwd, git_path }
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
    use std::fs;
    use std::path::Path;

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

    #[test]
    fn resolve_remove_target_uses_listed_path() {
        let entries = vec![
            GitWorktreeListEntry {
                path: PathBuf::from("/tmp/app"),
                head: None,
                branch: Some("main".into()),
                bare: false,
                detached: false,
            },
            GitWorktreeListEntry {
                path: PathBuf::from("/tmp/app/.worktrees/feat"),
                head: None,
                branch: Some("feature/x".into()),
                bare: false,
                detached: false,
            },
        ];
        let target = resolve_worktree_remove_target(
            &entries,
            Path::new("/tmp/app"),
            Path::new("/tmp/app/.worktrees/feat"),
        );
        assert_eq!(target.git_path, PathBuf::from("/tmp/app/.worktrees/feat"));
        assert_eq!(target.git_cwd, PathBuf::from("/tmp/app"));
    }

    #[test]
    fn resolve_remove_target_falls_back_when_unlisted() {
        let target = resolve_worktree_remove_target(
            &[],
            Path::new("/tmp/app"),
            Path::new("/tmp/app/.worktrees/feat"),
        );
        assert_eq!(target.git_path, PathBuf::from("/tmp/app/.worktrees/feat"));
        assert_eq!(target.git_cwd, PathBuf::from("/tmp/app"));
    }

    #[test]
    fn identity_same_path_matches_and_parent_differs() {
        let dir = std::env::temp_dir();
        let a = identity(&dir);
        let b = identity(&dir);
        assert!(a.is_some());
        assert!(same_identity(a.as_ref(), b.as_ref()));
        if let Some(parent) = dir.parent() {
            if parent != dir {
                let parent_id = identity(parent);
                if parent_id.is_some() {
                    assert!(!same_identity(a.as_ref(), parent_id.as_ref()));
                }
            }
        }
    }

    #[test]
    fn gitfile_vs_gitdir_classifies_linked_vs_main() {
        let root = std::env::temp_dir().join(format!(
            "ws-wt-kind-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let main = root.join("main");
        let linked = root.join("linked");
        fs::create_dir_all(&main).unwrap();
        fs::create_dir_all(&linked).unwrap();
        fs::create_dir_all(main.join(".git")).unwrap();
        fs::write(linked.join(".git"), "gitdir: ../main/.git/worktrees/x\n").unwrap();
        assert!(is_main_worktree_checkout(&main));
        assert!(!is_linked_worktree_checkout(&main));
        assert!(is_linked_worktree_checkout(&linked));
        assert!(!is_main_worktree_checkout(&linked));
        let _ = fs::remove_dir_all(&root);
    }
}

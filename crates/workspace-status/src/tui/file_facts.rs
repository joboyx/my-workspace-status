//! Disk facts of the dirty files of each checkout, read off the loop thread.
//!
//! The tree watch signature needs each dirty file's `size:mtimeMs`
//! ([`super::watch::file_disk_token`]), and the viewed marks need each
//! dirty file's content fingerprint ([`super::viewed::fingerprint_file_change`]).
//! Both read the disk. The worker that runs `git status` for one checkout
//! reads them for that checkout's dirty files ([`collect_repo_file_facts`])
//! and hands them to the reducer with the status result. The reducer only
//! looks them up in [`FileFactsCache`], so a status result never stats or
//! reads a worktree file on the loop thread.

use std::collections::HashMap;
use std::path::Path;

use crate::snapshot::{FileChange, WorkspaceSnapshot};

use super::viewed::{fingerprint_file_change, viewed_identity};
use super::watch::{file_disk_token, GONE_DISK_TOKEN};

/// Disk facts of one dirty file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileFacts {
    /// `size:mtimeMs`, or [`GONE_DISK_TOKEN`] when the file is missing.
    pub disk_token: String,
    /// Viewed-mark fingerprint of the file's status and bytes.
    pub fingerprint: String,
}

/// Disk facts of the dirty files of one checkout, by repo-relative path.
pub(crate) type RepoFileFacts = HashMap<String, FileFacts>;

/// Read the disk facts of each dirty file in `changes` of checkout `repo`.
///
/// Blocking: one `stat` and one read (at most
/// [`super::diff::HUGE_FILE_BYTES`]; a larger file gets two 64 KiB reads)
/// per file. Run it on a worker, or
/// once at launch before the loop starts.
pub(crate) fn collect_repo_file_facts(
    cwd: &Path,
    repo: &str,
    changes: &[FileChange],
) -> RepoFileFacts {
    changes
        .iter()
        .map(|change| {
            let facts = FileFacts {
                disk_token: file_disk_token(cwd, repo, &change.path),
                fingerprint: fingerprint_file_change(cwd, repo, change),
            };
            (change.path.clone(), facts)
        })
        .collect()
}

/// Disk facts of every checkout in the session snapshot, by checkout path.
///
/// Filled at launch ([`Self::from_disk`]) and then per checkout from each
/// status result ([`Self::set`]). Lookups never touch the disk.
#[derive(Clone, Debug, Default)]
pub(crate) struct FileFactsCache {
    by_repo: HashMap<String, RepoFileFacts>,
}

impl FileFactsCache {
    /// Read the facts of every dirty file in `snapshot`. Blocking; the
    /// launch calls it once before the loop starts.
    pub(crate) fn from_disk(cwd: &Path, snapshot: &WorkspaceSnapshot) -> Self {
        let by_repo = snapshot
            .repos
            .iter()
            .map(|repo| {
                (
                    repo.repo.clone(),
                    collect_repo_file_facts(cwd, &repo.repo, &repo.changes),
                )
            })
            .collect();
        Self { by_repo }
    }

    /// Replace the facts of checkout `repo`. `None` drops them (the
    /// checkout left the snapshot).
    pub(crate) fn set(&mut self, repo: &str, facts: Option<RepoFileFacts>) {
        match facts {
            Some(facts) => {
                self.by_repo.insert(repo.to_string(), facts);
            }
            None => {
                self.by_repo.remove(repo);
            }
        }
    }

    /// Facts of `path` in checkout `repo`, if a status result or the launch
    /// read them.
    pub(crate) fn get(&self, repo: &str, path: &str) -> Option<&FileFacts> {
        self.by_repo.get(repo).and_then(|files| files.get(path))
    }

    /// Disk token of `path` in checkout `repo`; [`GONE_DISK_TOKEN`] when
    /// no facts are known for it.
    pub(crate) fn disk_token(&self, repo: &str, path: &str) -> &str {
        self.get(repo, path)
            .map_or(GONE_DISK_TOKEN, |facts| facts.disk_token.as_str())
    }

    /// Viewed identity → fingerprint of each dirty file of `snapshot`
    /// that has facts. A file with no facts is left out.
    pub(crate) fn fingerprints(&self, snapshot: &WorkspaceSnapshot) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for repo in &snapshot.repos {
            for file in &repo.changes {
                if let Some(facts) = self.get(&repo.repo, &file.path) {
                    out.insert(
                        viewed_identity(&repo.repo, &file.path),
                        facts.fingerprint.clone(),
                    );
                }
            }
        }
        out
    }

    /// Viewed identity of each dirty file of `snapshot` that has no facts.
    /// Its fingerprint is unknown, which is not the same as changed.
    pub(crate) fn unknown_identities(&self, snapshot: &WorkspaceSnapshot) -> Vec<String> {
        snapshot
            .repos
            .iter()
            .flat_map(|repo| {
                repo.changes
                    .iter()
                    .filter(|file| self.get(&repo.repo, &file.path).is_none())
                    .map(|file| viewed_identity(&repo.repo, &file.path))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, RepoSnapshot};
    use crate::testutil::{repo_snapshot, unique_dir, unstaged_change};
    use std::fs;

    #[test]
    fn facts_match_the_disk_helpers_and_lookups_never_read() {
        let cwd = unique_dir("ws-file-facts");
        fs::create_dir_all(cwd.join("app")).unwrap();
        fs::write(cwd.join("app/a.txt"), "one\n").unwrap();
        let change = unstaged_change("a.txt");
        let repo = RepoSnapshot {
            has_unstaged: true,
            changes: vec![change.clone()],
            ..repo_snapshot("app")
        };
        let snapshot = build_workspace_snapshot(&[repo], &[], false, &[]);
        let cache = FileFactsCache::from_disk(&cwd, &snapshot);
        let facts = cache.get("app", "a.txt").expect("facts of the dirty file");
        assert_eq!(facts.disk_token, file_disk_token(&cwd, "app", "a.txt"));
        assert_eq!(
            facts.fingerprint,
            fingerprint_file_change(&cwd, "app", &change)
        );
        // The cache keeps what it read: the file is gone, the lookup is not.
        fs::remove_dir_all(&cwd).unwrap();
        assert_ne!(cache.disk_token("app", "a.txt"), GONE_DISK_TOKEN);
        assert_eq!(
            cache
                .fingerprints(&snapshot)
                .get(&viewed_identity("app", "a.txt")),
            Some(&facts.fingerprint)
        );
        assert_eq!(cache.disk_token("app", "b.txt"), GONE_DISK_TOKEN);
    }

    #[test]
    fn set_none_drops_a_checkout() {
        let mut cache = FileFactsCache::default();
        let mut files = RepoFileFacts::new();
        files.insert(
            "a.txt".into(),
            FileFacts {
                disk_token: "1:1".into(),
                fingerprint: "fp".into(),
            },
        );
        cache.set("app", Some(files));
        assert!(cache.get("app", "a.txt").is_some());
        cache.set("app", None);
        assert!(cache.get("app", "a.txt").is_none());
    }

    #[test]
    fn unknown_identities_are_the_dirty_files_without_facts() {
        let repo = |name: &str| RepoSnapshot {
            has_unstaged: true,
            changes: vec![unstaged_change("a.txt")],
            ..repo_snapshot(name)
        };
        let snapshot = build_workspace_snapshot(&[repo("app"), repo("lib")], &[], false, &[]);
        let mut cache = FileFactsCache::default();
        let mut files = RepoFileFacts::new();
        files.insert(
            "a.txt".into(),
            FileFacts {
                disk_token: "1:1".into(),
                fingerprint: "fp".into(),
            },
        );
        cache.set("app", Some(files));
        assert_eq!(
            cache.unknown_identities(&snapshot),
            vec![viewed_identity("lib", "a.txt")]
        );
        assert!(!cache
            .fingerprints(&snapshot)
            .contains_key(&viewed_identity("lib", "a.txt")));
    }
}

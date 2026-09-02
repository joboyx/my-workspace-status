//! Pasteable entity references for clipboard copy.

use super::store::repo_identity;

/// Git object a numbered diff is scoped to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffSource {
    /// Dirty worktree versus HEAD.
    Worktree,
    /// A commit object (`source: commit <sha>`).
    Commit {
        /// Full commit SHA.
        sha: String,
    },
    /// A stash entry (`source: stash <stash-ref>`).
    Stash {
        /// Stash ref (`stash@{n}`).
        stash_ref: String,
    },
}

/// Which side of a diff the focused rows belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSide {
    /// Only old/left numbers.
    Old,
    /// Only new/right numbers.
    New,
    /// Both sides, or neither (unified hunk).
    Unified,
}

/// Focused tree, graph, or diff entity for a clipboard reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityRef {
    /// Workspace root row. `path:` is the workspace cwd. No `repo:` line.
    Workspace {
        /// Workspace cwd.
        path: String,
    },
    /// File row in the workspace tree.
    File {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Repo-relative path.
        path: String,
    },
    /// Directory row in the workspace tree.
    Dir {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Repo-relative path.
        path: String,
    },
    /// Repo row (primary checkout).
    Repo {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Checkout path.
        path: String,
    },
    /// Family group row. `path:` is the family root checkout.
    Family {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Family root checkout path.
        path: String,
    },
    /// Linked or primary worktree row. `path:` is this checkout.
    Worktree {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// This checkout path.
        path: String,
    },
    /// Graph commit row.
    Commit {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Full commit SHA.
        sha: String,
    },
    /// Local branch.
    Branch {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Branch name.
        name: String,
    },
    /// Graph stash row.
    Stash {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Stash ref (`stash@{n}`).
        stash_ref: String,
        /// Stash commit id. Empty skips `sha:` and `short:`.
        sha: String,
        /// Stash subject. Empty skips `subject:`.
        subject: String,
    },
    /// Numbered diff row or visual line range.
    Diff {
        /// [`repo_identity`](super::store::repo_identity) for this checkout.
        repo: String,
        /// Repo-relative path.
        path: String,
        /// Inclusive start (1-based).
        start_line: u32,
        /// Inclusive end. Equal to [`Self::Diff::start_line`] for one line.
        end_line: u32,
        /// Worktree, commit, or stash source.
        source: DiffSource,
        /// Old, new, or unified side.
        side: DiffSide,
    },
}

impl EntityRef {
    /// Workspace row at `path` (cwd).
    pub fn workspace(path: impl Into<String>) -> Self {
        Self::Workspace { path: path.into() }
    }

    /// File row. `repo:` uses [`repo_identity`].
    pub fn file(checkout: &str, primary_repo: Option<&str>, path: impl Into<String>) -> Self {
        Self::File {
            repo: repo_identity(checkout, primary_repo),
            path: path.into(),
        }
    }

    /// Directory row. `repo:` uses [`repo_identity`].
    pub fn dir(checkout: &str, primary_repo: Option<&str>, path: impl Into<String>) -> Self {
        Self::Dir {
            repo: repo_identity(checkout, primary_repo),
            path: path.into(),
        }
    }

    /// Repo row. `repo:` uses [`repo_identity`]; `path` is the checkout.
    pub fn repo(checkout: &str, primary_repo: Option<&str>, path: impl Into<String>) -> Self {
        Self::Repo {
            repo: repo_identity(checkout, primary_repo),
            path: path.into(),
        }
    }

    /// Family row. `repo:` uses [`repo_identity`]; `path` is the family root.
    pub fn family(checkout: &str, primary_repo: Option<&str>, path: impl Into<String>) -> Self {
        Self::Family {
            repo: repo_identity(checkout, primary_repo),
            path: path.into(),
        }
    }

    /// Worktree row. `repo:` uses [`repo_identity`]; `path` is this checkout.
    pub fn worktree(checkout: &str, primary_repo: Option<&str>, path: impl Into<String>) -> Self {
        Self::Worktree {
            repo: repo_identity(checkout, primary_repo),
            path: path.into(),
        }
    }

    /// Commit row. `repo:` uses [`repo_identity`].
    pub fn commit(checkout: &str, primary_repo: Option<&str>, sha: impl Into<String>) -> Self {
        Self::Commit {
            repo: repo_identity(checkout, primary_repo),
            sha: sha.into(),
        }
    }

    /// Branch row. `repo:` uses [`repo_identity`].
    pub fn branch(checkout: &str, primary_repo: Option<&str>, name: impl Into<String>) -> Self {
        Self::Branch {
            repo: repo_identity(checkout, primary_repo),
            name: name.into(),
        }
    }

    /// Stash row. `repo:` uses [`repo_identity`]. Empty `sha` / `subject` are omitted on format.
    pub fn stash(
        checkout: &str,
        primary_repo: Option<&str>,
        stash_ref: impl Into<String>,
        sha: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        Self::Stash {
            repo: repo_identity(checkout, primary_repo),
            stash_ref: stash_ref.into(),
            sha: sha.into(),
            subject: subject.into(),
        }
    }

    /// Diff row or range. `repo:` uses [`repo_identity`].
    pub fn diff(
        checkout: &str,
        primary_repo: Option<&str>,
        path: impl Into<String>,
        start_line: u32,
        end_line: u32,
        source: DiffSource,
        side: DiffSide,
    ) -> Self {
        Self::Diff {
            repo: repo_identity(checkout, primary_repo),
            path: path.into(),
            start_line,
            end_line,
            source,
            side,
        }
    }
}

/// Format `entity` as one `key: value` block with a trailing newline.
///
/// No markdown fences. Line order is `kind`, `repo` (except workspace), then
/// the extra fields for that kind.
pub fn format_entity_reference(entity: &EntityRef) -> String {
    let mut out = String::new();
    match entity {
        EntityRef::Workspace { path } => {
            out.push_str("kind: workspace\n");
            push_field(&mut out, "path", path);
        }
        EntityRef::File { repo, path } => {
            out.push_str("kind: file\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "path", path);
        }
        EntityRef::Dir { repo, path } => {
            out.push_str("kind: dir\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "path", path);
        }
        EntityRef::Repo { repo, path } => {
            out.push_str("kind: repo\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "path", path);
        }
        EntityRef::Family { repo, path } => {
            out.push_str("kind: family\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "path", path);
        }
        EntityRef::Worktree { repo, path } => {
            out.push_str("kind: worktree\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "path", path);
        }
        EntityRef::Commit { repo, sha } => {
            out.push_str("kind: commit\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "sha", sha);
            push_field(&mut out, "short", short_sha(sha));
        }
        EntityRef::Branch { repo, name } => {
            out.push_str("kind: branch\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "ref", name);
        }
        EntityRef::Stash {
            repo,
            stash_ref,
            sha,
            subject,
        } => {
            out.push_str("kind: stash\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "ref", stash_ref);
            if !sha.is_empty() {
                push_field(&mut out, "sha", sha);
                push_field(&mut out, "short", short_sha(sha));
            }
            if !subject.is_empty() {
                push_field(&mut out, "subject", subject);
            }
        }
        EntityRef::Diff {
            repo,
            path,
            start_line,
            end_line,
            source,
            side,
        } => {
            out.push_str("kind: diff\n");
            push_field(&mut out, "repo", repo);
            push_field(&mut out, "path", path);
            push_field(&mut out, "lines", &line_span(*start_line, *end_line));
            push_field(&mut out, "source", &format_diff_source(source));
            push_field(&mut out, "side", diff_side_label(*side));
        }
    }
    out
}

fn push_field(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

fn short_sha(sha: &str) -> &str {
    if sha.len() > 12 {
        &sha[..12]
    } else {
        sha
    }
}

fn line_span(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

fn format_diff_source(source: &DiffSource) -> String {
    match source {
        DiffSource::Worktree => "worktree".to_string(),
        DiffSource::Commit { sha } => format!("commit {sha}"),
        DiffSource::Stash { stash_ref } => format!("stash {stash_ref}"),
    }
}

fn diff_side_label(side: DiffSide) -> &'static str {
    match side {
        DiffSide::Old => "old",
        DiffSide::New => "new",
        DiffSide::Unified => "unified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_workspace() {
        let entity = EntityRef::workspace("/tmp/ws");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: workspace\npath: /tmp/ws\n"
        );
    }

    #[test]
    fn format_file() {
        let entity = EntityRef::file("/tmp/app", None, "src/lib.rs");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: file\nrepo: /tmp/app\npath: src/lib.rs\n"
        );
    }

    #[test]
    fn format_dir() {
        let entity = EntityRef::dir("/tmp/app", None, "src");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: dir\nrepo: /tmp/app\npath: src\n"
        );
    }

    #[test]
    fn format_repo() {
        let entity = EntityRef::repo("/tmp/app", None, "/tmp/app");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: repo\nrepo: /tmp/app\npath: /tmp/app\n"
        );
    }

    #[test]
    fn format_family() {
        let entity = EntityRef::family("/tmp/app", None, "/tmp/app");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: family\nrepo: /tmp/app\npath: /tmp/app\n"
        );
    }

    #[test]
    fn format_worktree() {
        let entity = EntityRef::worktree("/tmp/app", None, "/tmp/app");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: worktree\nrepo: /tmp/app\npath: /tmp/app\n"
        );
    }

    #[test]
    fn format_commit_full_sha_and_short_12() {
        let entity = EntityRef::commit("/tmp/app", None, "abcdef1234567890abcd");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: commit\nrepo: /tmp/app\nsha: abcdef1234567890abcd\nshort: abcdef123456\n"
        );
    }

    #[test]
    fn format_commit_short_sha_uses_full_when_not_longer_than_12() {
        let entity = EntityRef::commit("/tmp/app", None, "deadbeef");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: commit\nrepo: /tmp/app\nsha: deadbeef\nshort: deadbeef\n"
        );
    }

    #[test]
    fn format_branch() {
        let entity = EntityRef::branch("/tmp/app", None, "main");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: branch\nrepo: /tmp/app\nref: main\n"
        );
    }

    #[test]
    fn format_stash_with_sha_and_subject() {
        let entity = EntityRef::stash(
            "/tmp/app",
            None,
            "stash@{0}",
            "deadbeef1234567890ab",
            "WIP on main: first",
        );
        assert_eq!(
            format_entity_reference(&entity),
            "kind: stash\nrepo: /tmp/app\nref: stash@{0}\nsha: deadbeef1234567890ab\nshort: deadbeef1234\nsubject: WIP on main: first\n"
        );
    }

    #[test]
    fn format_stash_skips_empty_sha_and_subject() {
        let entity = EntityRef::stash("/tmp/app", None, "stash@{1}", "", "");
        assert_eq!(
            format_entity_reference(&entity),
            "kind: stash\nrepo: /tmp/app\nref: stash@{1}\n"
        );
    }

    #[test]
    fn format_diff_single_line_worktree_new() {
        let entity = EntityRef::diff(
            "/tmp/app",
            None,
            "src/lib.rs",
            12,
            12,
            DiffSource::Worktree,
            DiffSide::New,
        );
        assert_eq!(
            format_entity_reference(&entity),
            "kind: diff\nrepo: /tmp/app\npath: src/lib.rs\nlines: 12\nsource: worktree\nside: new\n"
        );
    }

    #[test]
    fn format_diff_range_commit_old() {
        let entity = EntityRef::diff(
            "/tmp/app",
            None,
            "src/lib.rs",
            10,
            20,
            DiffSource::Commit {
                sha: "abcdef1234567890abcd".into(),
            },
            DiffSide::Old,
        );
        assert_eq!(
            format_entity_reference(&entity),
            "kind: diff\nrepo: /tmp/app\npath: src/lib.rs\nlines: 10-20\nsource: commit abcdef1234567890abcd\nside: old\n"
        );
    }

    #[test]
    fn format_diff_stash_unified() {
        let entity = EntityRef::diff(
            "/tmp/app",
            None,
            "README.md",
            1,
            3,
            DiffSource::Stash {
                stash_ref: "stash@{2}".into(),
            },
            DiffSide::Unified,
        );
        assert_eq!(
            format_entity_reference(&entity),
            "kind: diff\nrepo: /tmp/app\npath: README.md\nlines: 1-3\nsource: stash stash@{2}\nside: unified\n"
        );
    }

    #[test]
    fn linked_worktree_file_repo_is_primary_not_sibling() {
        let entity = EntityRef::file("/tmp/feature-wt", Some("/tmp/app"), "src/lib.rs");
        let out = format_entity_reference(&entity);
        assert_eq!(out, "kind: file\nrepo: /tmp/app\npath: src/lib.rs\n");
        assert!(
            !out.contains("feature-wt"),
            "sibling checkout must not appear as repo: {out}"
        );
    }

    #[test]
    fn linked_worktree_uses_repo_identity_normalization() {
        let entity = EntityRef::worktree("/tmp/feature-wt", Some("/tmp/app/"), "/tmp/feature-wt");
        let out = format_entity_reference(&entity);
        assert_eq!(
            out,
            "kind: worktree\nrepo: /tmp/app\npath: /tmp/feature-wt\n"
        );
        assert!(
            !out.lines().any(|line| line == "repo: /tmp/feature-wt"),
            "linked worktree must not emit the sibling path as repo: {out}"
        );
    }
}

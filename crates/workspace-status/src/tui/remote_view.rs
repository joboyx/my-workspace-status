//! "View remotely": the forge web page of a focused entity.
//!
//! A [`RemoteTarget`] names what the operator focused (the repository, a
//! branch, a commit, a tag, a file or directory). [`resolve`] picks the
//! remote, reads its forge, and checks that the entity is on that remote; the
//! result is a [`RemotePage`], and [`page_url`] turns it into the URL.
//!
//! Everything here is pure except [`resolve`]. It only composes read-only
//! probes from [`crate::git`] (the git subprocesses live there), and it
//! blocks: a tag check asks the remote over the network. Callers run it on a
//! worker thread, never on the TTY event thread.
//!
//! The status strings are plain text with no backticks, like
//! [`super::pull_request::no_pr_status`].

use std::path::Path;

use super::pull_request::{commit_url, parse_remote, percent_encode, Forge, ForgeRepo};
use crate::git;

/// Status line when the checkout has no usable remote.
pub const NO_REMOTE: &str = "no remote for this checkout";

/// Status line when the browser could not be started.
pub const OPEN_FAILED: &str = "could not open browser";

/// Status line for a row with uncommitted changes.
pub const UNCOMMITTED_LOCAL_ONLY: &str = "uncommitted changes are local only";

/// Status line for a row with no entity that has a web page.
pub const NOTHING_TO_VIEW: &str = "nothing to view remotely on this row";

/// Status line when remote `remote` is not on GitHub or GitLab.
pub fn not_forge_status(remote: &str) -> String {
    format!("{remote} is not a GitHub or GitLab remote")
}

/// Status line when commit `sha` is on no branch of `remote`. Shows the
/// short id.
pub fn commit_not_on_remote_status(sha: &str, remote: &str) -> String {
    format!("commit {} is not on {remote}", short_sha(sha))
}

/// Status line when `branch` has no remote-tracking branch on `remote`.
pub fn branch_not_on_remote_status(branch: &str, remote: &str) -> String {
    format!("branch {branch} is not on {remote}")
}

/// Status line when `remote` answered that it has no tag `tag`.
pub fn tag_not_on_remote_status(tag: &str, remote: &str) -> String {
    format!("tag {tag} is not on {remote}")
}

/// Status line when `remote` could not be asked about tag `tag`.
pub fn tag_check_failed_status(tag: &str, remote: &str) -> String {
    format!("could not check tag {tag} on {remote}")
}

/// Status line when `path` is not in the remote revision `place`, for
/// example `origin/feature` or `origin at abc1234`.
pub fn file_not_on_remote_status(path: &str, place: &str) -> String {
    format!("file {path} is not on {place}")
}

/// Status line for a stash entry such as `stash@{0}`.
pub fn stash_local_only_status(stash_ref: &str) -> String {
    format!("{stash_ref} is local only")
}

/// First seven characters of a commit id, the short id on screen.
fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The revision a file or directory page opens at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileRev {
    /// A local branch; the page opens at its name on the remote.
    Branch(String),
    /// A full commit id; the page opens at that commit.
    Commit(String),
}

/// What the operator wants to see on the forge, before any remote check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteTarget {
    /// The repository home page.
    Repo,
    /// A local branch, by its local name.
    Branch(String),
    /// A remote-tracking branch by its short name, for example
    /// `origin/feature`.
    RemoteBranch(String),
    /// A commit, by its full id.
    Commit(String),
    /// A tag, by its name without `refs/tags/`.
    Tag(String),
    /// A file or directory, by its path from the repository root.
    File {
        /// The revision the page opens at.
        rev: FileRev,
        /// Path from the repository root, `/`-separated; empty for the root.
        path: String,
        /// 1-based line for the `#L<n>` anchor (files only).
        line: Option<u32>,
        /// True for a directory page, false for a file page.
        dir: bool,
    },
}

/// A page that is known to exist on the remote, ready for [`page_url`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemotePage {
    /// The repository home page.
    Repo,
    /// The file tree of a branch, by its name on the remote.
    Branch(String),
    /// A commit page, by its full id.
    Commit(String),
    /// A tag page (GitHub release page, GitLab tag page).
    Tag(String),
    /// A file at `rev` (a remote branch name or a commit id).
    File {
        /// Remote branch name or full commit id.
        rev: String,
        /// Path from the repository root.
        path: String,
        /// 1-based line for the `#L<n>` anchor.
        line: Option<u32>,
    },
    /// A directory at `rev` (a remote branch name or a commit id).
    Dir {
        /// Remote branch name or full commit id.
        rev: String,
        /// Path from the repository root; empty for the root tree.
        path: String,
    },
}

/// Web URL of `page` in `repo`.
///
/// GitHub paths go right after `https://{host}/{path}`; GitLab puts `-/`
/// in between. Branch, tag, revision and file path are encoded with
/// [`encode_path`], so a branch such as `feature/x` keeps its `/`.
///
/// | Page | GitHub | GitLab |
/// | --- | --- | --- |
/// | repo | `/` (none) | `/` (none) |
/// | branch | `/tree/<b>` | `/-/tree/<b>` |
/// | commit | `/commit/<sha>` | `/-/commit/<sha>` |
/// | tag | `/releases/tag/<t>` | `/-/tags/<t>` |
/// | file | `/blob/<rev>/<path>[#L<n>]` | `/-/blob/<rev>/<path>[#L<n>]` |
/// | dir | `/tree/<rev>/<path>` | `/-/tree/<rev>/<path>` |
pub fn page_url(repo: &ForgeRepo, page: &RemotePage) -> String {
    let base = format!("https://{}/{}", repo.host, repo.path);
    let sep = match repo.forge {
        Forge::GitHub => "",
        Forge::GitLab => "-/",
    };
    match page {
        RemotePage::Repo => base,
        RemotePage::Branch(branch) => format!("{base}/{sep}tree/{}", encode_path(branch)),
        RemotePage::Commit(sha) => commit_url(repo, sha),
        RemotePage::Tag(tag) => match repo.forge {
            Forge::GitHub => format!("{base}/releases/tag/{}", encode_path(tag)),
            Forge::GitLab => format!("{base}/-/tags/{}", encode_path(tag)),
        },
        RemotePage::File { rev, path, line } => {
            let anchor = line.map(|n| format!("#L{n}")).unwrap_or_default();
            format!(
                "{base}/{sep}blob/{}/{}{anchor}",
                encode_path(rev),
                encode_path(path)
            )
        }
        RemotePage::Dir { rev, path } if path.is_empty() => {
            format!("{base}/{sep}tree/{}", encode_path(rev))
        }
        RemotePage::Dir { rev, path } => format!(
            "{base}/{sep}tree/{}/{}",
            encode_path(rev),
            encode_path(path)
        ),
    }
}

/// Percent-encode each `/`-separated segment of `path` and keep the `/`.
///
/// Every byte except RFC 3986 unreserved characters (letters, digits, `-`,
/// `.`, `_`, `~`) is encoded, so a space is `%20`, `#` is `%23`, and a
/// non-ASCII character is its UTF-8 bytes (`é` is `%C3%A9`).
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(percent_encode)
        .collect::<Vec<_>>()
        .join("/")
}

/// Pick the remote for `target`, read its forge, and check that the entity
/// is on that remote. `Err` is the status line to show.
///
/// The remote: for a branch (or a file at a branch), the branch's push
/// remote; for a remote-tracking branch, the remote its name starts with;
/// otherwise the push remote of the current branch
/// ([`git::checkout_remote_name`]).
///
/// Checks: a branch needs `refs/remotes/<remote>/<name on remote>`; a commit
/// must be in some branch of the remote; a file or directory must exist in
/// that remote revision; a tag is asked of the remote itself
/// ([`git::remote_has_tag`], the one network call). Every other check reads
/// local refs only, as of the last fetch or push. Blocks.
pub fn resolve(checkout: &Path, target: &RemoteTarget) -> Result<(ForgeRepo, RemotePage), String> {
    let (remote, tracked) = match target {
        RemoteTarget::RemoteBranch(short_ref) => {
            let (remote, branch) =
                git::split_remote_ref(checkout, short_ref).ok_or_else(|| NO_REMOTE.to_string())?;
            (remote, Some(branch))
        }
        RemoteTarget::Branch(branch)
        | RemoteTarget::File {
            rev: FileRev::Branch(branch),
            ..
        } => (git::push_remote_name(checkout, branch), None),
        _ => (git::checkout_remote_name(checkout), None),
    };
    let url = git::remote_url(checkout, &remote).ok_or_else(|| NO_REMOTE.to_string())?;
    let repo = parse_remote(&url).ok_or_else(|| not_forge_status(&remote))?;
    let page = match target {
        RemoteTarget::Repo => RemotePage::Repo,
        RemoteTarget::Branch(branch) => {
            RemotePage::Branch(remote_branch(checkout, &remote, branch)?)
        }
        RemoteTarget::RemoteBranch(short_ref) => {
            let branch = tracked.unwrap_or_default();
            if !git::remote_branch_exists(checkout, &remote, &branch) {
                return Err(branch_not_on_remote_status(short_ref, &remote));
            }
            RemotePage::Branch(branch)
        }
        RemoteTarget::Commit(sha) => {
            remote_commit(checkout, &remote, sha)?;
            RemotePage::Commit(sha.clone())
        }
        RemoteTarget::Tag(tag) => match git::remote_has_tag(checkout, &remote, tag) {
            Some(true) => RemotePage::Tag(tag.clone()),
            Some(false) => return Err(tag_not_on_remote_status(tag, &remote)),
            None => return Err(tag_check_failed_status(tag, &remote)),
        },
        RemoteTarget::File {
            rev,
            path,
            line,
            dir,
        } => {
            let (rev, spec, place) = match rev {
                FileRev::Branch(branch) => {
                    let name = remote_branch(checkout, &remote, branch)?;
                    let spec = format!("refs/remotes/{remote}/{name}");
                    let place = format!("{remote}/{name}");
                    (name, spec, place)
                }
                FileRev::Commit(sha) => {
                    remote_commit(checkout, &remote, sha)?;
                    let place = format!("{remote} at {}", short_sha(sha));
                    (sha.clone(), sha.clone(), place)
                }
            };
            if !git::rev_has_path(checkout, &spec, path) {
                return Err(file_not_on_remote_status(path, &place));
            }
            let path = path.clone();
            if *dir {
                RemotePage::Dir { rev, path }
            } else {
                RemotePage::File {
                    rev,
                    path,
                    line: *line,
                }
            }
        }
    };
    Ok((repo, page))
}

/// Name of local `branch` on `remote`, or the "not on remote" status.
fn remote_branch(checkout: &Path, remote: &str, branch: &str) -> Result<String, String> {
    let name = git::remote_branch_name(checkout, branch, remote);
    if git::remote_branch_exists(checkout, remote, &name) {
        Ok(name)
    } else {
        Err(branch_not_on_remote_status(branch, remote))
    }
}

/// `Ok` when commit `sha` is on `remote`, else the "not on remote" status.
fn remote_commit(checkout: &Path, remote: &str, sha: &str) -> Result<(), String> {
    if git::commit_on_remote(checkout, remote, sha) {
        Ok(())
    } else {
        Err(commit_not_on_remote_status(sha, remote))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{git, init_repo, unique_dir};
    use std::fs;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn github() -> ForgeRepo {
        parse_remote("git@github.com:octo/demo.git").unwrap()
    }

    fn gitlab() -> ForgeRepo {
        parse_remote("https://gitlab.example.com/group/sub/demo.git").unwrap()
    }

    fn file(rev: &str, path: &str, line: Option<u32>) -> RemotePage {
        RemotePage::File {
            rev: rev.into(),
            path: path.into(),
            line,
        }
    }

    fn dir(rev: &str, path: &str) -> RemotePage {
        RemotePage::Dir {
            rev: rev.into(),
            path: path.into(),
        }
    }

    // ---- page_url ----

    #[test]
    fn page_url_github_pages() {
        let base = "https://github.com/octo/demo";
        let cases = [
            (RemotePage::Repo, base.to_string()),
            (
                RemotePage::Branch("main".into()),
                format!("{base}/tree/main"),
            ),
            (
                RemotePage::Branch("feature/x".into()),
                format!("{base}/tree/feature/x"),
            ),
            (
                RemotePage::Commit(SHA.into()),
                format!("{base}/commit/{SHA}"),
            ),
            (
                RemotePage::Tag("v1.0".into()),
                format!("{base}/releases/tag/v1.0"),
            ),
            (
                file("main", "src/a.rs", Some(42)),
                format!("{base}/blob/main/src/a.rs#L42"),
            ),
            (
                file(SHA, "src/a.rs", None),
                format!("{base}/blob/{SHA}/src/a.rs"),
            ),
            (
                dir("feature/x", "src"),
                format!("{base}/tree/feature/x/src"),
            ),
            (dir("main", ""), format!("{base}/tree/main")),
        ];
        for (page, want) in cases {
            assert_eq!(page_url(&github(), &page), want, "{page:?}");
        }
    }

    #[test]
    fn page_url_gitlab_inserts_dash_after_subgroup_path() {
        let base = "https://gitlab.example.com/group/sub/demo";
        let cases = [
            (RemotePage::Repo, base.to_string()),
            (
                RemotePage::Branch("feature/x".into()),
                format!("{base}/-/tree/feature/x"),
            ),
            (
                RemotePage::Commit(SHA.into()),
                format!("{base}/-/commit/{SHA}"),
            ),
            (
                RemotePage::Tag("v1.0".into()),
                format!("{base}/-/tags/v1.0"),
            ),
            (
                file("main", "src/a.rs", Some(7)),
                format!("{base}/-/blob/main/src/a.rs#L7"),
            ),
            (dir("main", "src/ui"), format!("{base}/-/tree/main/src/ui")),
        ];
        for (page, want) in cases {
            assert_eq!(page_url(&gitlab(), &page), want, "{page:?}");
        }
    }

    #[test]
    fn page_url_encodes_branch_tag_and_path_segments() {
        let base = "https://github.com/octo/demo";
        let cases = [
            (
                file("feat/50% off", "docs/my notes#1.md", Some(3)),
                format!("{base}/blob/feat/50%25%20off/docs/my%20notes%231.md#L3"),
            ),
            (
                file("main", "src/café/ü.rs", None),
                format!("{base}/blob/main/src/caf%C3%A9/%C3%BC.rs"),
            ),
            (
                RemotePage::Tag("release/v1+rc".into()),
                format!("{base}/releases/tag/release/v1%2Brc"),
            ),
            (
                RemotePage::Branch("a b".into()),
                format!("{base}/tree/a%20b"),
            ),
        ];
        for (page, want) in cases {
            assert_eq!(page_url(&github(), &page), want, "{page:?}");
        }
    }

    #[test]
    fn encode_path_keeps_slashes_and_unreserved() {
        assert_eq!(encode_path("a/b-c_d.e~f/G9"), "a/b-c_d.e~f/G9");
        assert_eq!(encode_path("a b/#/%"), "a%20b/%23/%25");
        assert_eq!(encode_path(""), "");
    }

    #[test]
    fn status_lines_are_plain_text() {
        assert_eq!(
            commit_not_on_remote_status(SHA, "origin"),
            "commit 0123456 is not on origin"
        );
        assert_eq!(
            branch_not_on_remote_status("feature", "origin"),
            "branch feature is not on origin"
        );
        assert_eq!(
            tag_not_on_remote_status("v1", "origin"),
            "tag v1 is not on origin"
        );
        assert_eq!(
            file_not_on_remote_status("src/a.rs", "origin/feature"),
            "file src/a.rs is not on origin/feature"
        );
        assert_eq!(
            not_forge_status("origin"),
            "origin is not a GitHub or GitLab remote"
        );
        assert_eq!(
            tag_check_failed_status("v1", "origin"),
            "could not check tag v1 on origin"
        );
        assert_eq!(
            stash_local_only_status("stash@{0}"),
            "stash@{0} is local only"
        );
        for line in [
            NO_REMOTE,
            OPEN_FAILED,
            UNCOMMITTED_LOCAL_ONLY,
            NOTHING_TO_VIEW,
        ] {
            assert!(!line.contains('`'), "{line}");
        }
    }

    // ---- resolve ----

    /// Repo with a GitHub-looking `origin` and `refs/remotes/origin/main` at
    /// the seed commit (a stand-in for a push). The host is not
    /// `github.com`, so a host `insteadOf` rewrite cannot change the URL.
    fn repo_with_origin(prefix: &str) -> std::path::PathBuf {
        let dir = unique_dir(prefix);
        init_repo(&dir);
        let url = "git@github.example.invalid:octo/demo.git";
        git(&dir, &["remote", "add", "origin", url]);
        git(&dir, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        dir
    }

    fn head(dir: &Path) -> String {
        crate::git::exec_git(&["rev-parse", "HEAD"], dir)
    }

    #[test]
    fn resolve_refuses_without_remote_or_forge() {
        let dir = unique_dir("ws-remote-view-none");
        init_repo(&dir);
        assert_eq!(
            resolve(&dir, &RemoteTarget::Repo),
            Err(NO_REMOTE.to_string())
        );
        git(&dir, &["remote", "add", "origin", "/srv/git/demo.git"]);
        assert_eq!(
            resolve(&dir, &RemoteTarget::Repo),
            Err(not_forge_status("origin"))
        );
        // The forge check comes first, so a tag on such a remote never
        // reaches ls-remote.
        assert_eq!(
            resolve(&dir, &RemoteTarget::Tag("v1".into())),
            Err(not_forge_status("origin"))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_branch_commit_and_file_pages() {
        let dir = repo_with_origin("ws-remote-view-resolve");
        let repo = parse_remote("git@github.example.invalid:octo/demo.git").unwrap();
        let pushed = head(&dir);
        assert_eq!(
            resolve(&dir, &RemoteTarget::Repo),
            Ok((repo.clone(), RemotePage::Repo))
        );
        assert_eq!(
            resolve(&dir, &RemoteTarget::Branch("main".into())),
            Ok((repo.clone(), RemotePage::Branch("main".into())))
        );
        assert_eq!(
            resolve(&dir, &RemoteTarget::RemoteBranch("origin/main".into())),
            Ok((repo.clone(), RemotePage::Branch("main".into())))
        );
        assert_eq!(
            resolve(&dir, &RemoteTarget::Commit(pushed.clone())),
            Ok((repo.clone(), RemotePage::Commit(pushed.clone())))
        );
        let readme = |rev: FileRev, line| RemoteTarget::File {
            rev,
            path: "README.md".into(),
            line,
            dir: false,
        };
        assert_eq!(
            resolve(&dir, &readme(FileRev::Branch("main".into()), Some(1))),
            Ok((repo.clone(), file("main", "README.md", Some(1))))
        );
        assert_eq!(
            resolve(&dir, &readme(FileRev::Commit(pushed.clone()), None)),
            Ok((repo.clone(), file(&pushed, "README.md", None)))
        );
        let root = RemoteTarget::File {
            rev: FileRev::Branch("main".into()),
            path: String::new(),
            line: None,
            dir: true,
        };
        assert_eq!(resolve(&dir, &root), Ok((repo.clone(), dir_page("main"))));

        // Local work that was never pushed.
        fs::write(dir.join("new.rs"), "fn main() {}\n").unwrap();
        git(&dir, &["add", "new.rs"]);
        git(&dir, &["commit", "-q", "-m", "local"]);
        let local = head(&dir);
        assert_eq!(
            resolve(&dir, &RemoteTarget::Commit(local.clone())),
            Err(commit_not_on_remote_status(&local, "origin"))
        );
        let new_file = RemoteTarget::File {
            rev: FileRev::Branch("main".into()),
            path: "new.rs".into(),
            line: None,
            dir: false,
        };
        assert_eq!(
            resolve(&dir, &new_file),
            Err(file_not_on_remote_status("new.rs", "origin/main"))
        );
        git(&dir, &["branch", "feature"]);
        assert_eq!(
            resolve(&dir, &RemoteTarget::Branch("feature".into())),
            Err(branch_not_on_remote_status("feature", "origin"))
        );
        assert_eq!(
            resolve(&dir, &RemoteTarget::RemoteBranch("upstream/main".into())),
            Err(NO_REMOTE.to_string())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn dir_page(rev: &str) -> RemotePage {
        dir(rev, "")
    }
}

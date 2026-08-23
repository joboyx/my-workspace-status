//! Load a [`workspace_status_graph::GraphModel`] from git + the snapshot.
//!
//! History matches Ink `gitLogGraphWindow`: `log --exclude=refs/stash --all
//! --topo-order --date-order --skip --max-count`. Default window is 300.
//! Missing `stash^1` parents are fetched with `log --no-walk` and appended
//! after the log prefix so autoload skip stays on `window`, not `commits.len()`.

use std::collections::HashSet;
use std::path::Path;

use workspace_status_graph::{
    Commit, GraphModel, GraphRef, Stash, SyncState, SyncStatus, Worktree, DEFAULT_GRAPH_WINDOW,
};

use crate::git::exec_git;
use crate::snapshot::{CheckoutKind, WorkspaceRepoSnapshot, WorkspaceSnapshot};

/// Identity used to keep graph scroll when the same row is still focused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphIdentity {
    pub repo: String,
    pub head: String,
}

/// Load commits, stash, worktrees, and sync for `repo`.
pub fn load_graph_model(
    cwd: &Path,
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    show_ignored: bool,
) -> (GraphModel, GraphIdentity) {
    load_graph_model_window(cwd, snapshot, repo, show_ignored, 0, DEFAULT_GRAPH_WINDOW)
}

/// Load one `git log` page. `skip` / `limit` map to `--skip` / `--max-count`.
pub fn load_graph_model_window(
    cwd: &Path,
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    show_ignored: bool,
    skip: usize,
    limit: usize,
) -> (GraphModel, GraphIdentity) {
    let repo_dir = cwd.join(repo);
    let Some(row) = snapshot.repos.iter().find(|r| r.repo == repo) else {
        return (
            GraphModel {
                skip,
                limit,
                uncommitted: Some(false),
                ..GraphModel::default()
            },
            GraphIdentity {
                repo: repo.into(),
                head: String::new(),
            },
        );
    };

    let head = exec_git(&["rev-parse", "HEAD"], &repo_dir);
    let refs = load_refs(&repo_dir);
    let (window_commits, truncated) = load_commits(&repo_dir, skip, limit, &refs);
    let stashes = load_stashes(&repo_dir);
    let extra = load_missing_stash_parents(&repo_dir, &window_commits, &stashes, &refs);
    let window = window_commits.len();
    let mut commits = window_commits;
    commits.extend(extra);
    let worktrees = load_worktrees(snapshot, row, &head, show_ignored);
    let has_changes = row.has_unstaged || row.has_staged || row.has_untracked;
    let model = GraphModel {
        commits,
        stashes,
        worktrees,
        head_id: if head.is_empty() {
            None
        } else {
            Some(head.clone())
        },
        sync: Some(sync_from_snapshot(row)),
        show_ignored,
        uncommitted: Some(has_changes),
        skip,
        limit,
        has_more: truncated,
        window,
    };
    (
        model,
        GraphIdentity {
            repo: repo.to_string(),
            head,
        },
    )
}

/// True when the graph cursor is on the last loaded row and older history exists.
pub fn should_autoload(args: ShouldAutoload) -> bool {
    if !args.has_more || args.loading || args.loaded_count == 0 {
        return false;
    }
    args.cursor_index >= args.loaded_count - 1
}

/// Inputs for [`should_autoload`].
#[derive(Clone, Copy, Debug)]
pub struct ShouldAutoload {
    pub cursor_index: usize,
    pub loaded_count: usize,
    pub has_more: bool,
    pub loading: bool,
}

/// Merge the next log page into `current`. Skip stays at the original window
/// start. Extra stash parents stay after the log prefix.
pub fn merge_autoload(current: &GraphModel, page: GraphModel) -> GraphModel {
    let window_count = current.window_count();
    let page_window = page.window_count();
    let cur_window = &current.commits[..window_count.min(current.commits.len())];
    let cur_extras = &current.commits[window_count.min(current.commits.len())..];
    let page_window_commits = &page.commits[..page_window.min(page.commits.len())];
    let page_extras = &page.commits[page_window.min(page.commits.len())..];

    let mut seen_window: HashSet<&str> = cur_window.iter().map(|c| c.id.as_str()).collect();
    let mut merged_window: Vec<Commit> = cur_window.to_vec();
    for commit in page_window_commits {
        if seen_window.insert(commit.id.as_str()) {
            merged_window.push(commit.clone());
        }
    }
    let seen_all: HashSet<String> = merged_window.iter().map(|c| c.id.clone()).collect();
    let mut extras: Vec<Commit> = Vec::new();
    for commit in cur_extras.iter().chain(page_extras) {
        if seen_all.contains(&commit.id) {
            continue;
        }
        if let Some(prev) = extras.iter_mut().find(|c| c.id == commit.id) {
            for parent in &commit.parents {
                if !prev.parents.contains(parent) {
                    prev.parents.push(parent.clone());
                }
            }
        } else {
            extras.push(commit.clone());
        }
    }
    let window = merged_window.len();
    let mut commits = merged_window;
    commits.extend(extras);

    GraphModel {
        commits,
        stashes: page.stashes,
        worktrees: page.worktrees,
        head_id: page.head_id,
        sync: page.sync,
        show_ignored: current.show_ignored,
        uncommitted: page.uncommitted.or(current.uncommitted),
        skip: current.skip,
        limit: autoload_limit(current),
        has_more: page.has_more,
        window,
    }
}

/// Next `git log` skip for autoload: original skip plus the log-window prefix.
pub fn autoload_skip(current: &GraphModel) -> usize {
    current.skip + current.window_count()
}

/// Page size for the next autoload fetch.
pub fn autoload_limit(current: &GraphModel) -> usize {
    if current.limit == 0 {
        DEFAULT_GRAPH_WINDOW
    } else {
        current.limit
    }
}

/// `--max-count` for a same-repo refresh: keep already-loaded history.
pub fn refresh_graph_limit(current: Option<&GraphModel>) -> usize {
    current
        .map(|g| g.window_count().max(g.limit).max(DEFAULT_GRAPH_WINDOW))
        .unwrap_or(DEFAULT_GRAPH_WINDOW)
}

fn load_commits(
    repo_dir: &Path,
    skip: usize,
    limit: usize,
    refs: &[(String, GraphRef)],
) -> (Vec<Commit>, bool) {
    let skip_arg = format!("--skip={skip}");
    let max_arg = format!("--max-count={limit}");
    let raw = exec_git(
        &[
            "log",
            // `--exclude` must precede `--all` or stash still gets included.
            "--exclude=refs/stash",
            "--all",
            "--topo-order",
            "--date-order",
            &skip_arg,
            &max_arg,
            "--pretty=format:%H%x00%P%x00%s%x00%an%x00%at",
        ],
        repo_dir,
    );
    let commits: Vec<Commit> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| parse_commit_line(line, refs))
        .collect();
    let truncated = limit > 0 && commits.len() == limit;
    (commits, truncated)
}

fn load_missing_stash_parents(
    repo_dir: &Path,
    window_commits: &[Commit],
    stashes: &[Stash],
    refs: &[(String, GraphRef)],
) -> Vec<Commit> {
    let mut in_window: HashSet<String> = window_commits.iter().map(|c| c.id.clone()).collect();
    let missing: Vec<String> = stashes
        .iter()
        .filter_map(|stash| stash.parent_id.clone())
        .filter(|id| !id.is_empty() && !in_window.contains(id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if missing.is_empty() {
        return Vec::new();
    }
    load_commits_by_ids(repo_dir, &missing, refs)
        .into_iter()
        .filter(|commit| in_window.insert(commit.id.clone()))
        .collect()
}

fn load_commits_by_ids(
    repo_dir: &Path,
    ids: &[String],
    refs: &[(String, GraphRef)],
) -> Vec<Commit> {
    let unique: Vec<String> = ids
        .iter()
        .filter(|id| !id.is_empty())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if unique.is_empty() {
        return Vec::new();
    }
    let mut args: Vec<String> = vec![
        "log".into(),
        "--no-walk".into(),
        "--ignore-missing".into(),
        "--pretty=format:%H%x00%P%x00%s%x00%an%x00%at".into(),
    ];
    args.extend(unique);
    let refs_args: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = exec_git(&refs_args, repo_dir);
    raw.lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| parse_commit_line(line, refs))
        .collect()
}

fn parse_commit_line(line: &str, refs: &[(String, GraphRef)]) -> Option<Commit> {
    let mut parts = line.split('\0');
    let id = parts.next()?.to_string();
    if id.is_empty() {
        return None;
    }
    let parents = parts
        .next()
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let subject = parts.next().unwrap_or("").to_string();
    let author_name = parts.next().unwrap_or("").to_string();
    let author_date_unix = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    let commit_refs = refs
        .iter()
        .filter(|(sha, _)| sha == &id)
        .map(|(_, graph_ref)| graph_ref.clone())
        .collect();
    Some(Commit {
        id,
        subject,
        parents,
        refs: commit_refs,
        author_name,
        author_date_unix,
    })
}

fn classify_ref(refname: &str, short: &str) -> Option<GraphRef> {
    if short.ends_with("/HEAD") {
        return None;
    }
    if refname.starts_with("refs/heads/") {
        Some(GraphRef::local(short))
    } else if refname.starts_with("refs/remotes/") {
        Some(GraphRef::remote(short))
    } else if refname.starts_with("refs/tags/") {
        Some(GraphRef::tag(short))
    } else {
        None
    }
}

fn load_refs(repo_dir: &Path) -> Vec<(String, GraphRef)> {
    let raw = exec_git(
        &[
            "for-each-ref",
            "--format=%(objectname)%00%(refname)%00%(refname:short)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
        repo_dir,
    );
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let sha = parts.next()?;
            let refname = parts.next()?;
            let short = parts.next().unwrap_or("");
            let graph_ref = classify_ref(refname, short)?;
            Some((sha.to_string(), graph_ref))
        })
        .collect()
}

fn load_stashes(repo_dir: &Path) -> Vec<Stash> {
    let raw = exec_git(
        &[
            "stash",
            "list",
            "--format=%gd%x00%H%x00%P%x00%s%x00%at%x00%an",
        ],
        repo_dir,
    );
    raw.lines()
        .filter(|l| !l.is_empty())
        .filter_map(parse_stash_line)
        .collect()
}

fn parse_stash_line(line: &str) -> Option<Stash> {
    let mut parts = line.split('\0');
    let stash_ref = parts.next()?.to_string();
    let id = parts.next()?.to_string();
    if stash_ref.is_empty() || id.is_empty() {
        return None;
    }
    let parent = parts
        .next()
        .and_then(|p| p.split_whitespace().next())
        .map(str::to_string);
    let subject = parts.next().unwrap_or("").to_string();
    let author_date_unix = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    let author_name = parts.next().unwrap_or("").to_string();
    Some(Stash {
        id,
        stash_ref,
        subject,
        author_name,
        author_date_unix,
        parent_id: parent.filter(|s| !s.is_empty()),
    })
}

fn load_worktrees(
    snapshot: &WorkspaceSnapshot,
    focused: &WorkspaceRepoSnapshot,
    head: &str,
    show_ignored: bool,
) -> Vec<Worktree> {
    let primary = focused.primary_repo.as_deref().unwrap_or(&focused.repo);
    snapshot
        .repos
        .iter()
        .filter(|repo| {
            repo.checkout_kind == CheckoutKind::Linked
                && repo.primary_repo.as_deref() == Some(primary)
                && (show_ignored || !repo.ignored)
        })
        .map(|repo| Worktree {
            path: repo.repo.clone(),
            head_id: if repo.repo == focused.repo && !head.is_empty() {
                Some(head.to_string())
            } else {
                None
            },
            branch: Some(repo.branch.clone()),
            ignored: repo.ignored,
            is_current: repo.repo == focused.repo,
        })
        .collect()
}

fn sync_from_snapshot(repo: &WorkspaceRepoSnapshot) -> SyncState {
    let (ahead, behind) = parse_ahead_behind(&repo.sync_note);
    SyncState {
        branch: repo.branch.clone(),
        status: match repo.sync_status {
            crate::snapshot::SyncStatus::UpToDate => SyncStatus::UpToDate,
            crate::snapshot::SyncStatus::NoUpstream => SyncStatus::NoUpstream,
            crate::snapshot::SyncStatus::Ahead => SyncStatus::Ahead,
            crate::snapshot::SyncStatus::Behind => SyncStatus::Behind,
            crate::snapshot::SyncStatus::Diverged => SyncStatus::Diverged,
        },
        ahead,
        behind,
    }
}

fn parse_ahead_behind(note: &str) -> (u32, u32) {
    let ahead = capture(note, "ahead ");
    let ahead = if ahead == 0 {
        capture(note, "ahead by ")
    } else {
        ahead
    };
    let behind = capture(note, "behind ");
    let behind = if behind == 0 {
        capture(note, "behind by ")
    } else {
        behind
    };
    (ahead, behind)
}

fn capture(note: &str, label: &str) -> u32 {
    note.split(label)
        .nth(1)
        .and_then(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diverged_counts() {
        assert_eq!(parse_ahead_behind("diverged (ahead 2, behind 1)"), (2, 1));
        assert_eq!(parse_ahead_behind("ahead by 3 commits"), (3, 0));
        assert_eq!(parse_ahead_behind("behind by 4 commits"), (0, 4));
    }

    #[test]
    fn classify_ref_kinds() {
        assert_eq!(
            classify_ref("refs/heads/main", "main"),
            Some(GraphRef::local("main"))
        );
        assert_eq!(
            classify_ref("refs/remotes/origin/main", "origin/main"),
            Some(GraphRef::remote("origin/main"))
        );
        assert_eq!(
            classify_ref("refs/remotes/upstream/main", "upstream/main"),
            Some(GraphRef::remote("upstream/main"))
        );
        assert_eq!(
            classify_ref("refs/tags/v1.0", "v1.0"),
            Some(GraphRef::tag("v1.0"))
        );
        assert_eq!(
            classify_ref("refs/remotes/origin/HEAD", "origin/HEAD"),
            None
        );
    }

    #[test]
    fn parse_commit_line_loads_author_and_date() {
        let line = "abc123\0parent1 parent2\0fix login\0Ada Lovelace\x001700000000";
        let commit = parse_commit_line(line, &[]).expect("commit");
        assert_eq!(commit.id, "abc123");
        assert_eq!(commit.parents, vec!["parent1", "parent2"]);
        assert_eq!(commit.subject, "fix login");
        assert_eq!(commit.author_name, "Ada Lovelace");
        assert_eq!(commit.author_date_unix, 1_700_000_000);
    }

    #[test]
    #[test]
    fn parse_stash_line_loads_id_date_and_author() {
        let line =
            "stash@{1}\0s1abcdef\0parentsha othersha\0WIP on main\x001700000000\0Ada Lovelace";
        let stash = parse_stash_line(line).expect("stash");
        assert_eq!(stash.stash_ref, "stash@{1}");
        assert_eq!(stash.id, "s1abcdef");
        assert_eq!(stash.parent_id.as_deref(), Some("parentsha"));
        assert_eq!(stash.subject, "WIP on main");
        assert_eq!(stash.author_date_unix, 1_700_000_000);
        assert_eq!(stash.author_name, "Ada Lovelace");
    }

    #[test]
    fn parse_stash_line_skips_missing_id() {
        assert!(parse_stash_line("stash@{0}\0\0parent\0wip\x001\0Ada").is_none());
    }

    #[test]
    fn default_window_is_300() {
        assert_eq!(DEFAULT_GRAPH_WINDOW, 300);
    }

    #[test]
    fn should_autoload_only_at_end_when_has_more() {
        assert!(should_autoload(ShouldAutoload {
            cursor_index: 9,
            loaded_count: 10,
            has_more: true,
            loading: false,
        }));
        assert!(!should_autoload(ShouldAutoload {
            cursor_index: 8,
            loaded_count: 10,
            has_more: true,
            loading: false,
        }));
        assert!(!should_autoload(ShouldAutoload {
            cursor_index: 9,
            loaded_count: 10,
            has_more: false,
            loading: false,
        }));
        assert!(!should_autoload(ShouldAutoload {
            cursor_index: 9,
            loaded_count: 10,
            has_more: true,
            loading: true,
        }));
    }

    fn empty_commit(id: &str) -> Commit {
        Commit {
            id: id.into(),
            subject: id.into(),
            parents: Vec::new(),
            refs: Vec::new(),
            author_name: String::new(),
            author_date_unix: 0,
        }
    }

    #[test]
    fn merge_autoload_uses_window_not_commits_len() {
        let extra = empty_commit("stash-parent");
        let current = GraphModel {
            commits: vec![empty_commit("w0"), empty_commit("w1"), extra.clone()],
            skip: 0,
            limit: 2,
            has_more: true,
            window: 2,
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let page = GraphModel {
            commits: vec![empty_commit("win-2")],
            skip: 2,
            limit: 2,
            has_more: false,
            window: 1,
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        assert_eq!(autoload_skip(&current), 2);
        let merged = merge_autoload(&current, page);
        assert_eq!(merged.window, 3);
        assert!(!merged.has_more);
        assert_eq!(merged.skip, 0);
        assert_eq!(merged.limit, 2);
        assert_eq!(
            merged
                .commits
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["w0", "w1", "win-2", "stash-parent"]
        );
        assert_eq!(autoload_skip(&merged), 3);
        assert_eq!(refresh_graph_limit(Some(&merged)), DEFAULT_GRAPH_WINDOW);
        let grown = GraphModel {
            window: 400,
            limit: 300,
            ..merged
        };
        assert_eq!(refresh_graph_limit(Some(&grown)), 400);
    }
}

#[cfg(test)]
mod live_git {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};

    fn git_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("GIT_AUTHOR_NAME", "workspace-status graph-load"),
            (
                "GIT_AUTHOR_EMAIL",
                "workspace-status-graph-load@example.invalid",
            ),
            ("GIT_COMMITTER_NAME", "workspace-status graph-load"),
            (
                "GIT_COMMITTER_EMAIL",
                "workspace-status-graph-load@example.invalid",
            ),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ]
    }

    fn git(cwd: &Path, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(cwd);
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        let out = cmd.output().expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn temp_workspace() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ws-graph-load-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo = workspace.join("app");
        fs::create_dir_all(&repo).unwrap();
        (root, repo)
    }

    fn init_repo(dir: &Path) {
        let init = Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .status();
        if init.map(|s| s.success()).unwrap_or(false) == false {
            git(dir, &["init", "-q"]);
            git(dir, &["checkout", "-q", "-b", "main"]);
        }
        git(dir, &["config", "user.name", "workspace-status graph-load"]);
        git(
            dir,
            &[
                "config",
                "user.email",
                "workspace-status-graph-load@example.invalid",
            ],
        );
    }

    fn commit_file(dir: &Path, name: &str, contents: &str, message: &str) {
        fs::write(dir.join(name), contents).unwrap();
        git(dir, &["add", name]);
        git(dir, &["commit", "-q", "-m", message]);
    }

    fn snapshot_app() -> crate::snapshot::WorkspaceSnapshot {
        build_workspace_snapshot(
            &[RepoSnapshot {
                repo: "app".into(),
                branch: "main".into(),
                sync_status: SyncStatus::NoUpstream,
                sync_note: String::new(),
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
                merged_into_default: None,
                default_branch_override: None,
            }],
            &[],
            false,
            &[],
        )
    }

    #[test]
    fn log_all_includes_other_branch_tips() {
        let (root, repo) = temp_workspace();
        init_repo(&repo);
        commit_file(&repo, "a.txt", "1\n", "c1");
        git(&repo, &["checkout", "-q", "-b", "feature"]);
        commit_file(&repo, "a.txt", "feat\n", "c-feature");
        git(&repo, &["checkout", "-q", "main"]);
        commit_file(&repo, "a.txt", "2\n", "c2-main");
        let snapshot = snapshot_app();
        let (model, _) = load_graph_model_window(
            root.join("workspace").as_path(),
            &snapshot,
            "app",
            false,
            0,
            50,
        );
        let subjects: Vec<&str> = model.commits.iter().map(|c| c.subject.as_str()).collect();
        assert!(
            subjects.iter().any(|s| *s == "c-feature"),
            "other-branch tip must appear under --all, got {subjects:?}"
        );
        assert!(subjects.iter().any(|s| *s == "c2-main"), "{subjects:?}");
        assert!(!model.has_more);
        assert_eq!(model.skip, 0);
        assert_eq!(model.limit, 50);
        assert_eq!(model.window, model.commits.len());
        assert_eq!(model.uncommitted, Some(false));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_limit_sets_has_more() {
        let (root, repo) = temp_workspace();
        init_repo(&repo);
        for i in 0..6 {
            commit_file(&repo, "a.txt", &format!("{i}\n"), &format!("c{i}"));
        }
        let snapshot = snapshot_app();
        let cwd = root.join("workspace");
        let (page, _) = load_graph_model_window(&cwd, &snapshot, "app", false, 0, 3);
        assert_eq!(page.window, 3);
        assert_eq!(page.commits.len(), 3);
        assert!(page.has_more);
        let (page2, _) = load_graph_model_window(&cwd, &snapshot, "app", false, 3, 3);
        assert_eq!(page2.skip, 3);
        assert_ne!(page2.commits[0].id, page.commits[0].id);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stash_wip_is_excluded_from_log_window() {
        let (root, repo) = temp_workspace();
        init_repo(&repo);
        commit_file(&repo, "a.txt", "1\n", "c1");
        fs::write(repo.join("a.txt"), "dirty\n").unwrap();
        git(&repo, &["stash", "push", "-q", "-m", "wip-dirty"]);
        let snapshot = snapshot_app();
        let (model, _) = load_graph_model_window(
            root.join("workspace").as_path(),
            &snapshot,
            "app",
            false,
            0,
            50,
        );
        assert!(model
            .stashes
            .iter()
            .any(|s| s.stash_ref.starts_with("stash@{")));
        for commit in &model.commits {
            assert!(
                !commit.subject.starts_with("WIP on ")
                    && !commit.subject.starts_with("index on ")
                    && !commit.subject.starts_with("untracked files on "),
                "stash component leaked into graph: {}",
                commit.subject
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fetches_missing_stash_parent_without_changing_has_more() {
        let (root, repo) = temp_workspace();
        init_repo(&repo);
        commit_file(&repo, "f.txt", "base\n", "old-parent");
        fs::write(repo.join("f.txt"), "base\nstash-me\n").unwrap();
        git(
            &repo,
            &["stash", "push", "-q", "-m", "wip-old", "--", "f.txt"],
        );
        for i in 0..8 {
            commit_file(&repo, "f.txt", &format!("c{i}\n"), &format!("c{i}"));
        }
        let snapshot = snapshot_app();
        let (model, _) = load_graph_model_window(
            root.join("workspace").as_path(),
            &snapshot,
            "app",
            false,
            0,
            3,
        );
        assert_eq!(model.limit, 3);
        assert!(model.has_more);
        assert_eq!(model.window, 3);
        assert!(!model.stashes.is_empty());
        let parent_id = model.stashes[0].parent_id.clone().expect("stash^1");
        let window_ids: HashSet<&str> = model
            .commits
            .iter()
            .take(model.window)
            .map(|c| c.id.as_str())
            .collect();
        assert!(
            !window_ids.contains(parent_id.as_str()),
            "fixture parent must sit outside the log window"
        );
        assert!(
            model.commits.iter().any(|c| c.id == parent_id),
            "stash^1 outside the log window must be loaded so the tip can park"
        );
        assert!(model.commits.len() > 3);
        let _ = fs::remove_dir_all(&root);
    }
}

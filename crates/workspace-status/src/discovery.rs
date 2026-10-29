//! Discover git repos and collect snapshots.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use crate::config::{default_branch_override_for, WorkspaceStatusConfig};
use crate::git::{
    classify_upstream_rewrite, exec_git, exec_git_checked, head_equals_ref, is_ancestor,
    list_local_branches, list_worktrees_porcelain, resolve_default_branch_name,
    resolve_default_branch_tip_ref, rev_parse_quiet,
};
use crate::helpers::{
    is_default_branch, path_is_under_any, DETACHED_HEAD_BRANCH, STATUS_FAILED_NOTE,
    UNKNOWN_HEAD_BRANCH,
};
use crate::parallel::map_with_concurrency;
use crate::snapshot::{CheckoutKind, FileChange, RepoSnapshot, SyncStatus};
use crate::worktrees::{
    classify_merged_into_default, is_main_worktree_checkout, linked_worktrees_under_cwd,
    parse_worktree_list_porcelain,
};

#[derive(Debug, Clone)]
pub struct RepoCheckoutMeta {
    pub checkout_kind: CheckoutKind,
    pub primary_repo: Option<String>,
}

fn path_depth(repo_path: &str) -> usize {
    repo_path
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .count()
}

fn can_reach_only_repo(repo_path: &str, only_repos: &BTreeSet<String>) -> bool {
    for only in only_repos {
        if only == repo_path {
            return true;
        }
        if only.starts_with(&format!("{repo_path}/")) {
            return true;
        }
    }
    false
}

fn has_git_dir(dir: &Path) -> bool {
    match fs::metadata(dir.join(".git")) {
        Ok(st) => st.is_dir() || st.is_file(),
        Err(_) => false,
    }
}

fn is_main_checkout(repo_dir: &Path) -> bool {
    is_main_worktree_checkout(repo_dir)
}

fn find_main_checkout_rel(cwd: &Path, rel_path: &str) -> Option<String> {
    let cwd_abs = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut abs = cwd.join(rel_path);
    loop {
        if (abs == cwd_abs || abs.starts_with(&cwd_abs)) && is_main_worktree_checkout(&abs) {
            let rel = abs
                .strip_prefix(&cwd_abs)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            return Some(rel);
        }
        match abs.parent() {
            Some(parent) if parent != abs => abs = parent.to_path_buf(),
            _ => break,
        }
    }
    None
}

fn is_effective_directory(parent: &Path, name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    let full = parent.join(name);
    match fs::symlink_metadata(&full) {
        Ok(meta) if meta.is_dir() => true,
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::metadata(&full).map(|m| m.is_dir()).unwrap_or(false)
        }
        _ => false,
    }
}

fn posix_join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchState {
    pub branch: String,
    pub sync_status: SyncStatus,
    pub sync_note: String,
    /// Upstream ref from the header (`origin/main`), when the branch has one.
    pub upstream: Option<String>,
}

/// Parse the `##` porcelain branch header.
pub fn parse_branch_line(line: &str) -> Option<BranchState> {
    let value = line.strip_prefix("## ")?;
    if let Some(branch) = value.strip_prefix("No commits yet on ") {
        let branch = branch.trim();
        if branch.is_empty() {
            return None;
        }
        return Some(BranchState {
            branch: branch.to_string(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: "no commits yet".to_string(),
            upstream: None,
        });
    }
    if value == "HEAD (no branch)" {
        return Some(BranchState {
            branch: DETACHED_HEAD_BRANCH.to_string(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            upstream: None,
        });
    }

    // `branch...upstream [ahead N, behind M]`
    let (branch_and_rest, tracking) = if let Some((left, right)) = value.split_once(" [") {
        let tracking = right.strip_suffix(']').unwrap_or(right);
        (left, tracking)
    } else {
        (value, "")
    };
    let (branch, upstream) = if let Some((b, u)) = branch_and_rest.split_once("...") {
        (b, Some(u.split_whitespace().next().unwrap_or(u)))
    } else {
        (branch_and_rest, None)
    };
    if branch.is_empty() {
        return None;
    }
    if upstream.is_none() {
        return Some(BranchState {
            branch: branch.to_string(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            upstream: None,
        });
    }
    let upstream = upstream.map(str::to_string);
    let ahead = capture_count(tracking, "ahead ");
    let behind = capture_count(tracking, "behind ");
    if ahead > 0 && behind > 0 {
        return Some(BranchState {
            branch: branch.to_string(),
            sync_status: SyncStatus::Diverged,
            sync_note: format!("diverged (ahead {ahead}, behind {behind})"),
            upstream,
        });
    }
    if behind > 0 {
        return Some(BranchState {
            branch: branch.to_string(),
            sync_status: SyncStatus::Behind,
            sync_note: format!("behind by {behind} commits"),
            upstream,
        });
    }
    if ahead > 0 {
        return Some(BranchState {
            branch: branch.to_string(),
            sync_status: SyncStatus::Ahead,
            sync_note: format!("ahead by {ahead} commits"),
            upstream,
        });
    }
    Some(BranchState {
        branch: branch.to_string(),
        sync_status: SyncStatus::UpToDate,
        sync_note: String::new(),
        upstream,
    })
}

fn capture_count(tracking: &str, label: &str) -> u32 {
    tracking
        .split(label)
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

fn failed_repo_snapshot(
    repo_path: &str,
    override_name: Option<&str>,
    meta: &RepoCheckoutMeta,
) -> RepoSnapshot {
    RepoSnapshot {
        repo: repo_path.to_string(),
        branch: UNKNOWN_HEAD_BRANCH.to_string(),
        sync_status: SyncStatus::NoUpstream,
        sync_note: STATUS_FAILED_NOTE.to_string(),
        upstream_rewrite: None,
        head: String::new(),
        has_unstaged: false,
        has_staged: false,
        has_untracked: false,
        changes: Vec::new(),
        checkout_kind: meta.checkout_kind,
        primary_repo: meta.primary_repo.clone(),
        merged_into_default: None,
        default_branch_override: override_name.map(str::to_string),
        default_tip_ref: None,
        local_branches: Vec::new(),
    }
}

fn normalize_porcelain_status(status: char) -> Option<char> {
    match status {
        'R' | 'C' => Some('R'),
        'A' | 'M' | 'D' | 'U' => Some(status),
        'T' => Some('M'),
        _ => None,
    }
}

fn is_unmerged_xy(xy: &str) -> bool {
    xy.contains('U') || xy == "AA" || xy == "DD"
}

#[derive(Debug, Default)]
pub struct PorcelainChanges {
    pub staged: Vec<(char, String, Option<String>)>,
    pub unstaged: Vec<(char, String, Option<String>)>,
    pub untracked: Vec<String>,
}

fn parse_tracked_file_part(status: char, file_part: &str) -> (String, Option<String>) {
    if status == 'R' {
        if let Some((old, new)) = file_part.split_once(" -> ") {
            return (new.to_string(), Some(old.to_string()));
        }
    }
    (file_part.to_string(), None)
}

/// Parse porcelain=v1 file lines (no `##` header).
pub fn parse_porcelain_change_lines(lines: &[&str]) -> PorcelainChanges {
    let mut out = PorcelainChanges::default();
    for line in lines {
        if line.len() < 3 {
            continue;
        }
        let xy = &line[..2];
        let file_part = &line[3..];
        if xy == "??" {
            out.untracked.push(file_part.to_string());
            continue;
        }
        if is_unmerged_xy(xy) {
            let (path, old) = parse_tracked_file_part('U', file_part);
            out.unstaged.push(('U', path, old));
            continue;
        }
        let chars: Vec<char> = xy.chars().collect();
        if let Some(st) = chars.first().copied().and_then(normalize_porcelain_status) {
            let (path, old) = parse_tracked_file_part(st, file_part);
            out.staged.push((st, path, old));
        }
        if let Some(st) = chars.get(1).copied().and_then(normalize_porcelain_status) {
            let (path, old) = parse_tracked_file_part(st, file_part);
            out.unstaged.push((st, path, old));
        }
    }
    out
}

fn merge_file_changes(changes: &PorcelainChanges) -> Vec<FileChange> {
    let mut by_path: BTreeMap<String, FileChange> = BTreeMap::new();
    for (status, path, old) in &changes.staged {
        let entry = by_path.entry(path.clone()).or_insert_with(|| FileChange {
            path: path.clone(),
            staged_status: None,
            unstaged_status: None,
            untracked: false,
            old_path: None,
        });
        entry.staged_status = Some(status.to_string());
        if old.is_some() {
            entry.old_path = old.clone();
        }
    }
    for (status, path, old) in &changes.unstaged {
        let entry = by_path.entry(path.clone()).or_insert_with(|| FileChange {
            path: path.clone(),
            staged_status: None,
            unstaged_status: None,
            untracked: false,
            old_path: None,
        });
        entry.unstaged_status = Some(status.to_string());
        if entry.old_path.is_none() {
            entry.old_path = old.clone();
        }
    }
    for path in &changes.untracked {
        let entry = by_path.entry(path.clone()).or_insert_with(|| FileChange {
            path: path.clone(),
            staged_status: None,
            unstaged_status: None,
            untracked: false,
            old_path: None,
        });
        entry.untracked = true;
    }
    by_path.into_values().collect()
}

/// Paths the walk must skip when no repo is named: `ignoredPaths` always,
/// plus `hiddenPaths` unless the caller cleared them to show hidden repos.
fn excluded_paths(config: &WorkspaceStatusConfig) -> Vec<String> {
    config
        .hidden_paths
        .iter()
        .chain(config.ignored_paths.iter())
        .cloned()
        .collect()
}

fn should_include_repo(
    repo_path: &str,
    excluded: &[String],
    only_repos: Option<&BTreeSet<String>>,
) -> bool {
    if let Some(only) = only_repos {
        return only.contains(repo_path);
    }
    !path_is_under_any(repo_path, excluded)
}

/// Walk `cwd` up to `maxDepth` and return the workspace-relative primary
/// repos, sorted.
///
/// With no named repos (`only_repos` is `None`), the walk never descends
/// into or returns a path at or under an `ignoredPaths` or `hiddenPaths`
/// entry (clear `hidden_paths` first to find hidden repos). With named
/// repos, the walk only heads toward those names and both lists are
/// ignored: a named repo wins.
pub fn find_repos_with_config(
    cwd: &Path,
    config: &WorkspaceStatusConfig,
    only_repos: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let excluded = excluded_paths(config);
    let mut dirs = Vec::new();
    walk(cwd, "", config.max_depth, &excluded, only_repos, &mut dirs);
    dirs.sort();
    dirs
}

fn walk(
    cwd: &Path,
    rel_parent: &str,
    max_depth: u32,
    excluded: &[String],
    only_repos: Option<&BTreeSet<String>>,
    dirs: &mut Vec<String>,
) {
    let parent_depth = if rel_parent.is_empty() {
        0
    } else {
        path_depth(rel_parent)
    };
    if parent_depth as u32 >= max_depth {
        return;
    }
    let abs_parent = if rel_parent.is_empty() {
        cwd.to_path_buf()
    } else {
        cwd.join(rel_parent)
    };
    let Ok(entries) = fs::read_dir(&abs_parent) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        if !is_effective_directory(&abs_parent, &name) {
            continue;
        }
        let repo_path = posix_join(rel_parent, &name);
        if let Some(only) = only_repos {
            if !can_reach_only_repo(&repo_path, only) {
                continue;
            }
        } else if path_is_under_any(&repo_path, excluded) {
            continue;
        }
        let full = abs_parent.join(&name);
        if has_git_dir(&full) && should_include_repo(&repo_path, excluded, only_repos) {
            dirs.push(repo_path.clone());
        }
        walk(cwd, &repo_path, max_depth, excluded, only_repos, dirs);
    }
}

/// Whether discovery keeps a linked worktree at `linked_rel` of `primary_rel`.
///
/// With named repos, keep it when the worktree itself is named, or when its
/// primary is named and the worktree is not at or under an `ignored`
/// (`ignoredPaths`) entry: only a repo named on the command line wins over
/// `ignoredPaths`. `hiddenPaths` does not drop a named primary's worktrees.
/// Without named repos, drop it when it is at or under any `excluded` entry
/// (`ignoredPaths`, plus `hiddenPaths` when they were not cleared).
pub fn should_include_linked_worktree(
    linked_rel: &str,
    primary_rel: &str,
    excluded: &[String],
    ignored: &[String],
    only_repos: Option<&BTreeSet<String>>,
) -> bool {
    if let Some(only) = only_repos {
        return only.contains(linked_rel)
            || (only.contains(primary_rel) && !path_is_under_any(linked_rel, ignored));
    }
    !path_is_under_any(linked_rel, excluded)
}

pub fn expand_repos_with_linked_worktrees(
    cwd: &Path,
    walk_primaries: &[String],
    config: &WorkspaceStatusConfig,
    only_repos: Option<&BTreeSet<String>>,
) -> Vec<(String, RepoCheckoutMeta)> {
    let excluded = excluded_paths(config);
    let cwd_abs = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());

    let mut listing_roots: BTreeSet<String> = walk_primaries.iter().cloned().collect();
    if let Some(only) = only_repos {
        for filter in only {
            if walk_primaries.iter().any(|p| p == filter) {
                continue;
            }
            if let Some(main_rel) = find_main_checkout_rel(cwd, filter) {
                listing_roots.insert(main_rel);
            }
        }
    }

    let mut by_path: HashMap<String, RepoCheckoutMeta> = HashMap::new();
    for primary in walk_primaries {
        by_path.insert(
            primary.clone(),
            RepoCheckoutMeta {
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
            },
        );
    }

    for primary in &listing_roots {
        let primary_abs = cwd.join(primary);
        if !is_main_checkout(&primary_abs) {
            continue;
        }
        let porcelain = list_worktrees_porcelain(&primary_abs);
        if porcelain.is_empty() {
            continue;
        }
        let linked = linked_worktrees_under_cwd(
            &parse_worktree_list_porcelain(&porcelain),
            &cwd_abs,
            &primary_abs,
        );
        for (_abs, rel) in linked {
            if !should_include_linked_worktree(
                &rel,
                primary,
                &excluded,
                &config.ignored_paths,
                only_repos,
            ) {
                continue;
            }
            by_path.insert(
                rel.clone(),
                RepoCheckoutMeta {
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some(primary.clone()),
                },
            );
        }
    }

    let mut out: Vec<(String, RepoCheckoutMeta)> = by_path.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn override_for_path(
    repo_path: &str,
    primary_repo: Option<&str>,
    default_branches: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    default_branch_override_for(repo_path, default_branches)
        .or_else(|| primary_repo.and_then(|p| default_branch_override_for(p, default_branches)))
}

/// Merge-into-default for one checkout.
///
/// Same-commit as the default tip is open (`Some(false)`), not merged.
fn compute_merged_into_default(
    repo_dir: &Path,
    branch: &str,
    override_name: Option<&str>,
) -> Option<bool> {
    if is_default_branch(branch, override_name) {
        return None;
    }
    let default_branch = resolve_default_branch_name(repo_dir, override_name);
    if branch == default_branch {
        return None;
    }
    match resolve_default_branch_tip_ref(repo_dir, &default_branch) {
        None => classify_merged_into_default(branch, &default_branch, None, false),
        Some(tip) => classify_merged_into_default(
            branch,
            &default_branch,
            is_ancestor(repo_dir, "HEAD", &tip),
            head_equals_ref(repo_dir, &tip),
        ),
    }
}

pub fn process_repo(
    repo_path: &str,
    cwd: &Path,
    do_fetch: bool,
    override_name: Option<&str>,
    meta: &RepoCheckoutMeta,
) -> Option<RepoSnapshot> {
    let repo_dir = cwd.join(repo_path);
    if !has_git_dir(&repo_dir) {
        return None;
    }
    if do_fetch {
        let _ = exec_git_checked(&["fetch", "--quiet"], &repo_dir);
    }
    let porcelain = exec_git(
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "--ahead-behind",
            "--untracked-files=all",
        ],
        &repo_dir,
    );
    let lines: Vec<&str> = porcelain.lines().filter(|l| !l.is_empty()).collect();
    let Some(branch_state) = parse_branch_line(lines.first().copied().unwrap_or("")) else {
        return Some(failed_repo_snapshot(repo_path, override_name, meta));
    };
    let parsed = parse_porcelain_change_lines(&lines[1..]);
    let changes = merge_file_changes(&parsed);
    let has_unstaged = !parsed.unstaged.is_empty();
    let has_staged = !parsed.staged.is_empty();
    let has_untracked = !parsed.untracked.is_empty();
    let merged = compute_merged_into_default(&repo_dir, &branch_state.branch, override_name);
    let head = rev_parse_quiet("HEAD", &repo_dir).unwrap_or_default();
    let default_name = resolve_default_branch_name(&repo_dir, override_name);
    let default_tip_ref = resolve_default_branch_tip_ref(&repo_dir, &default_name);
    let local_branches = list_local_branches(&repo_dir)
        .into_iter()
        .map(|b| b.name)
        .collect();
    // Only a diverged branch can have its upstream rewritten under it.
    let upstream_rewrite = match (&branch_state.sync_status, &branch_state.upstream) {
        (SyncStatus::Diverged, Some(upstream)) => {
            classify_upstream_rewrite(&repo_dir, &branch_state.branch, upstream)
        }
        _ => None,
    };
    Some(RepoSnapshot {
        repo: repo_path.to_string(),
        branch: branch_state.branch,
        sync_status: branch_state.sync_status,
        sync_note: branch_state.sync_note,
        upstream_rewrite,
        head,
        has_unstaged,
        has_staged,
        has_untracked,
        changes,
        checkout_kind: meta.checkout_kind,
        primary_repo: meta.primary_repo.clone(),
        merged_into_default: merged,
        default_branch_override: override_name.map(str::to_string),
        default_tip_ref,
        local_branches,
    })
}

/// Discover checkouts (walk + linked worktrees) without running `git status`.
///
/// Each entry is `(path, checkout meta, default-branch override)`. Used by
/// the TTY loop to stream [`process_repo`] jobs after a cheap discover.
pub fn discover_checkouts(
    cwd: &Path,
    config: &WorkspaceStatusConfig,
    only_repos: Option<&BTreeSet<String>>,
) -> Vec<(String, RepoCheckoutMeta, Option<String>)> {
    let walk_primaries = find_repos_with_config(cwd, config, only_repos);
    let entries = expand_repos_with_linked_worktrees(cwd, &walk_primaries, config, only_repos);
    let default_branches = &config.default_branches;
    entries
        .into_iter()
        .map(|(repo_path, meta)| {
            let override_name =
                override_for_path(&repo_path, meta.primary_repo.as_deref(), default_branches);
            (repo_path, meta, override_name)
        })
        .collect()
}

/// Walk primaries and linked worktrees, then [`process_repo`] each checkout.
///
/// Independent checkouts run with at most `fetch_concurrency` in flight (the
/// resolved `fetchConcurrency`, default 10). Output order matches discovery
/// order.
pub fn collect_snapshots(
    cwd: &Path,
    do_fetch: bool,
    config: &WorkspaceStatusConfig,
    only_repos: Option<&BTreeSet<String>>,
    fetch_concurrency: usize,
) -> Vec<RepoSnapshot> {
    let entries = discover_checkouts(cwd, config, only_repos);
    let cwd = cwd.to_path_buf();
    map_with_concurrency(
        entries,
        fetch_concurrency,
        move |(repo_path, meta, override_name)| {
            process_repo(&repo_path, &cwd, do_fetch, override_name.as_deref(), &meta)
        },
    )
    .into_iter()
    .flatten()
    .collect()
}

/// Exit-style validation: `Err` lists the unknown repo name.
///
/// `loaded` is the merged config from `load_workspace_status_config`. Named
/// repos bypass `hiddenPaths` and `ignoredPaths`, so the walk here clears
/// both lists.
pub fn validate_filter_repos(
    cwd: &Path,
    loaded: &WorkspaceStatusConfig,
    filter_repos: &[String],
) -> Result<(), String> {
    if filter_repos.is_empty() {
        return Ok(());
    }
    let config = WorkspaceStatusConfig {
        hidden_paths: Vec::new(),
        ignored_paths: Vec::new(),
        ..loaded.clone()
    };
    let walk_primaries = find_repos_with_config(cwd, &config, None);
    let entries = expand_repos_with_linked_worktrees(cwd, &walk_primaries, &config, None);
    let known: BTreeSet<String> = entries.into_iter().map(|(p, _)| p).collect();
    for repo in filter_repos {
        if !known.contains(repo) {
            return Err(repo.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_branch_no_upstream() {
        let s = parse_branch_line("## main").unwrap();
        assert_eq!(s.branch, "main");
        assert_eq!(s.sync_status, SyncStatus::NoUpstream);
    }

    #[test]
    fn parse_branch_ahead_behind() {
        let s = parse_branch_line("## feature/x...origin/feature/x [ahead 2, behind 1]").unwrap();
        assert_eq!(s.sync_status, SyncStatus::Diverged);
        assert_eq!(s.sync_note, "diverged (ahead 2, behind 1)");
        assert_eq!(s.upstream.as_deref(), Some("origin/feature/x"));
    }

    #[test]
    fn parse_branch_keeps_the_upstream_name_only_with_an_upstream() {
        let up = parse_branch_line("## main...origin/main").unwrap();
        assert_eq!(up.upstream.as_deref(), Some("origin/main"));
        assert_eq!(parse_branch_line("## main").unwrap().upstream, None);
        assert_eq!(
            parse_branch_line("## No commits yet on main")
                .unwrap()
                .upstream,
            None
        );
    }

    #[test]
    fn parse_untracked_and_modified() {
        let parsed = parse_porcelain_change_lines(&[" M README.md", "?? new.txt"]);
        assert_eq!(parsed.unstaged[0].0, 'M');
        assert_eq!(parsed.untracked, vec!["new.txt"]);
    }
}

#[cfg(test)]
mod path_list_tests {
    use super::*;
    use crate::testutil::{git, init_repo, unique_dir};

    /// Workspace with repos at each relative path in `repos`.
    fn workspace(repos: &[&str]) -> std::path::PathBuf {
        let root = unique_dir("ws-discovery-paths");
        fs::create_dir_all(&root).unwrap();
        for repo in repos {
            init_repo(&root.join(repo));
        }
        root
    }

    fn config(hidden: &[&str], ignored: &[&str]) -> WorkspaceStatusConfig {
        WorkspaceStatusConfig {
            hidden_paths: hidden.iter().map(|s| s.to_string()).collect(),
            ignored_paths: ignored.iter().map(|s| s.to_string()).collect(),
            ..WorkspaceStatusConfig::with_defaults()
        }
    }

    fn found(
        root: &Path,
        config: &WorkspaceStatusConfig,
        only: Option<&BTreeSet<String>>,
    ) -> Vec<String> {
        discover_checkouts(root, config, only)
            .into_iter()
            .map(|(path, _, _)| path)
            .collect()
    }

    #[test]
    fn ignored_repo_folder_is_not_discovered() {
        let root = workspace(&["app", "notes"]);
        assert_eq!(found(&root, &config(&[], &["notes"]), None), vec!["app"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignored_folder_hides_every_repo_under_it() {
        let root = workspace(&["app", "tmp/a", "tmp/b/c"]);
        assert_eq!(
            found(&root, &config(&[], &[]), None),
            vec!["app", "tmp/a", "tmp/b/c"]
        );
        assert_eq!(found(&root, &config(&[], &["tmp"]), None), vec!["app"]);
        assert_eq!(found(&root, &config(&["tmp"], &[]), None), vec!["app"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn entry_matches_whole_segments_not_a_prefix() {
        let root = workspace(&["tmp", "tmpfoo", "app/tmp"]);
        assert_eq!(
            found(&root, &config(&["tmp"], &["tmp"]), None),
            vec!["app/tmp", "tmpfoo"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn linked_worktree_under_a_listed_path_is_dropped() {
        let root = workspace(&["app"]);
        fs::create_dir_all(root.join("tmp")).unwrap();
        git(
            &root.join("app"),
            &["worktree", "add", "-q", "-b", "feat", "../tmp/app-feat"],
        );
        assert_eq!(
            found(&root, &config(&[], &[]), None),
            vec!["app", "tmp/app-feat"]
        );
        assert_eq!(found(&root, &config(&[], &["tmp"]), None), vec!["app"]);
        assert_eq!(found(&root, &config(&["tmp"], &[]), None), vec!["app"]);
        let named: BTreeSet<String> = ["app".to_string()].into();
        assert_eq!(
            found(&root, &config(&[], &["tmp"]), Some(&named)),
            vec!["app"],
            "a named primary does not bring back a linked worktree under ignoredPaths"
        );
        assert_eq!(
            found(&root, &config(&["tmp"], &[]), Some(&named)),
            vec!["app", "tmp/app-feat"],
            "a named primary keeps its linked worktrees under hiddenPaths"
        );
        let named_linked: BTreeSet<String> = ["tmp/app-feat".to_string()].into();
        let loaded = config(&[], &["tmp"]);
        assert_eq!(
            found(&root, &loaded, Some(&named_linked)),
            vec!["tmp/app-feat"],
            "a linked worktree named itself wins over ignoredPaths"
        );
        assert_eq!(
            validate_filter_repos(&root, &loaded, &["tmp/app-feat".to_string()]),
            Ok(())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleared_hidden_list_still_excludes_ignored_paths() {
        let root = workspace(&["app", "notes", "tmp/a"]);
        let loaded = config(&["notes"], &["tmp"]);
        assert_eq!(found(&root, &loaded, None), vec!["app"]);
        let show_hidden = WorkspaceStatusConfig {
            hidden_paths: Vec::new(),
            ..loaded
        };
        assert_eq!(found(&root, &show_hidden, None), vec!["app", "notes"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn named_repo_under_listed_path_is_found_and_validates() {
        let root = workspace(&["app", "tmp/a", "notes"]);
        let loaded = config(&["notes"], &["tmp"]);
        let named: BTreeSet<String> = ["notes".to_string(), "tmp/a".to_string()].into();
        assert_eq!(found(&root, &loaded, Some(&named)), vec!["notes", "tmp/a"]);
        assert_eq!(
            validate_filter_repos(&root, &loaded, &["tmp/a".to_string(), "notes".to_string()]),
            Ok(())
        );
        assert_eq!(
            validate_filter_repos(&root, &loaded, &["tmp/missing".to_string()]),
            Err("tmp/missing".to_string())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignored_wins_over_hidden() {
        let root = workspace(&["tmp/a", "tmp/b", "notes"]);
        let loaded = config(&["tmp", "notes"], &["tmp/a", "notes"]);
        let show_hidden = WorkspaceStatusConfig {
            hidden_paths: Vec::new(),
            ..loaded
        };
        assert_eq!(found(&root, &show_hidden, None), vec!["tmp/b"]);
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod upstream_rewrite_tests {
    use super::*;
    use crate::testutil::{upstream_rewrite_fixture, RewriteFixture, RewriteKind};

    fn snapshot(fixture: &RewriteFixture) -> RepoSnapshot {
        let meta = RepoCheckoutMeta {
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
        };
        process_repo("checkout", &fixture.root, false, None, &meta).expect("repo snapshot")
    }

    #[test]
    fn dropped_commits_set_the_rewrite_and_keep_the_sync_note() {
        let fixture = upstream_rewrite_fixture(RewriteKind::Dropped);
        let snap = snapshot(&fixture);
        assert_eq!(snap.sync_status, SyncStatus::Diverged);
        assert_eq!(snap.sync_note, "diverged (ahead 2, behind 1)");
        let rewrite = snap.upstream_rewrite.expect("classified as a rewrite");
        assert_eq!(rewrite.upstream, "origin/main");
        assert_eq!(rewrite.dropped, 2);
        assert_eq!(rewrite.yours, 0);
        assert_eq!(rewrite.tip, fixture.local_tip);
    }

    #[test]
    fn a_branch_that_is_not_diverged_has_no_rewrite() {
        let fixture = upstream_rewrite_fixture(RewriteKind::Dropped);
        crate::testutil::git(&fixture.checkout, &["reset", "-q", "--hard", "origin/main"]);
        let snap = snapshot(&fixture);
        assert_eq!(snap.sync_status, SyncStatus::UpToDate);
        assert_eq!(snap.upstream_rewrite, None);
    }
}

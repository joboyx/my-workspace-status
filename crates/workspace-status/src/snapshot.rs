//! Workspace snapshot types and JSON serialization. Matches `docs/snapshot.md`.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::helpers::{
    compare_repo_paths_for_display, get_branch_emoji, get_branch_kind, get_branch_priority,
    get_sync_priority, is_attention_sync_note, is_default_branch, sorted_unique, BranchKind,
};

pub const WORKSPACE_SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncStatus {
    #[serde(rename = "up-to-date")]
    UpToDate,
    #[serde(rename = "no-upstream")]
    NoUpstream,
    Behind,
    Ahead,
    Diverged,
}

impl SyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "up-to-date",
            Self::NoUpstream => "no-upstream",
            Self::Behind => "behind",
            Self::Ahead => "ahead",
            Self::Diverged => "diverged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckoutKind {
    Primary,
    Linked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unstaged_status: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub untracked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone)]
pub struct RepoSnapshot {
    pub repo: String,
    pub branch: String,
    pub sync_status: SyncStatus,
    pub sync_note: String,
    /// `HEAD` sha. Live-watch identity only; not part of `--json`.
    pub head: String,
    pub has_unstaged: bool,
    pub has_staged: bool,
    pub has_untracked: bool,
    pub changes: Vec<FileChange>,
    pub checkout_kind: CheckoutKind,
    pub primary_repo: Option<String>,
    pub merged_into_default: Option<bool>,
    pub default_branch_override: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRepoSnapshot {
    pub repo: String,
    pub ignored: bool,
    pub branch: String,
    pub sync_status: SyncStatus,
    pub sync_note: String,
    /// `HEAD` sha. Live-watch identity only; omitted from `--json`.
    #[serde(skip)]
    pub head: String,
    pub checkout_kind: CheckoutKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_repo: Option<String>,
    pub merged_into_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch_override: Option<String>,
    pub has_unstaged: bool,
    pub has_staged: bool,
    pub has_untracked: bool,
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub version: u32,
    pub show_ignored: bool,
    pub filter_repos: Vec<String>,
    pub ignored_repos: Vec<String>,
    pub repos: Vec<WorkspaceRepoSnapshot>,
}

#[derive(Debug, Clone)]
pub struct SummaryState {
    pub changes_uncommitted: BTreeSet<String>,
    pub changes_staged: BTreeSet<String>,
    pub changes_both: BTreeSet<String>,
    pub changes_untracked: BTreeSet<String>,
    pub sync_behind: BTreeSet<String>,
    pub sync_ahead: BTreeSet<String>,
    pub sync_diverged: BTreeSet<String>,
    pub branch_feature: BTreeSet<String>,
    pub branch_bugfix: BTreeSet<String>,
    pub branch_chore: BTreeSet<String>,
    pub branch_release: BTreeSet<String>,
    pub branch_unknown: BTreeSet<String>,
    pub linked_worktrees: BTreeSet<String>,
}

impl SummaryState {
    fn empty() -> Self {
        Self {
            changes_uncommitted: BTreeSet::new(),
            changes_staged: BTreeSet::new(),
            changes_both: BTreeSet::new(),
            changes_untracked: BTreeSet::new(),
            sync_behind: BTreeSet::new(),
            sync_ahead: BTreeSet::new(),
            sync_diverged: BTreeSet::new(),
            branch_feature: BTreeSet::new(),
            branch_bugfix: BTreeSet::new(),
            branch_chore: BTreeSet::new(),
            branch_release: BTreeSet::new(),
            branch_unknown: BTreeSet::new(),
            linked_worktrees: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerboseRow {
    pub repo: String,
    pub branch: String,
    pub sync: String,
    pub files: String,
    pub note: String,
}

fn serialize_file_change(change: &FileChange) -> FileChange {
    FileChange {
        path: change.path.clone(),
        staged_status: change.staged_status.clone(),
        unstaged_status: change.unstaged_status.clone(),
        untracked: change.untracked,
        old_path: change.old_path.clone(),
    }
}

fn to_workspace_repo(snapshot: &RepoSnapshot, ignored: &BTreeSet<String>) -> WorkspaceRepoSnapshot {
    WorkspaceRepoSnapshot {
        repo: snapshot.repo.clone(),
        ignored: ignored.contains(&snapshot.repo),
        branch: snapshot.branch.clone(),
        sync_status: snapshot.sync_status,
        sync_note: snapshot.sync_note.clone(),
        head: snapshot.head.clone(),
        checkout_kind: snapshot.checkout_kind,
        primary_repo: snapshot.primary_repo.clone(),
        merged_into_default: snapshot.merged_into_default,
        default_branch_override: snapshot.default_branch_override.clone(),
        has_unstaged: snapshot.has_unstaged,
        has_staged: snapshot.has_staged,
        has_untracked: snapshot.has_untracked,
        changes: snapshot.changes.iter().map(serialize_file_change).collect(),
    }
}

pub fn build_workspace_snapshot(
    snapshots: &[RepoSnapshot],
    ignored_repos: &[String],
    show_ignored: bool,
    filter_repos: &[String],
) -> WorkspaceSnapshot {
    let ignored_repos = sorted_unique(ignored_repos.iter().cloned());
    let filter_repos = sorted_unique(filter_repos.iter().cloned());
    let ignored_set: BTreeSet<String> = ignored_repos.iter().cloned().collect();
    let mut repos: Vec<RepoSnapshot> = snapshots.to_vec();
    repos.sort_by(compare_repo_paths_for_display);
    WorkspaceSnapshot {
        version: WORKSPACE_SNAPSHOT_VERSION,
        show_ignored,
        filter_repos,
        ignored_repos,
        repos: repos
            .iter()
            .map(|s| to_workspace_repo(s, &ignored_set))
            .collect(),
    }
}

pub fn visible_workspace_snapshot(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot {
    let named: BTreeSet<&str> = snapshot.filter_repos.iter().map(String::as_str).collect();
    let repos = snapshot
        .repos
        .iter()
        .filter(|repo| snapshot.show_ignored || !repo.ignored || named.contains(repo.repo.as_str()))
        .cloned()
        .collect();
    WorkspaceSnapshot {
        version: snapshot.version,
        show_ignored: snapshot.show_ignored,
        filter_repos: snapshot.filter_repos.clone(),
        ignored_repos: snapshot.ignored_repos.clone(),
        repos,
    }
}

/// Replace or drop one checkout, then rebuild the workspace snapshot.
///
/// Repos that are not `repo` stay as they were (previous generation) until
/// a later result arrives. `None` removes that path.
pub fn replace_repo_in_snapshot(
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    next: Option<RepoSnapshot>,
    show_ignored: bool,
) -> WorkspaceSnapshot {
    let mut snaps = repo_snapshots_from_workspace(snapshot);
    match next {
        Some(row) => {
            if let Some(slot) = snaps.iter_mut().find(|r| r.repo == repo) {
                *slot = row;
            } else {
                snaps.push(row);
            }
        }
        None => {
            snaps.retain(|r| r.repo != repo);
        }
    }
    build_workspace_snapshot(
        &snaps,
        &snapshot.ignored_repos,
        show_ignored,
        &snapshot.filter_repos,
    )
}

pub fn repo_snapshots_from_workspace(snapshot: &WorkspaceSnapshot) -> Vec<RepoSnapshot> {
    snapshot
        .repos
        .iter()
        .map(|repo| RepoSnapshot {
            repo: repo.repo.clone(),
            branch: repo.branch.clone(),
            sync_status: repo.sync_status,
            sync_note: repo.sync_note.clone(),
            head: repo.head.clone(),
            has_unstaged: repo.has_unstaged,
            has_staged: repo.has_staged,
            has_untracked: repo.has_untracked,
            changes: repo.changes.clone(),
            checkout_kind: repo.checkout_kind,
            primary_repo: repo.primary_repo.clone(),
            merged_into_default: repo.merged_into_default,
            default_branch_override: repo.default_branch_override.clone(),
        })
        .collect()
}

pub fn serialize_workspace_snapshot(snapshot: &WorkspaceSnapshot) -> String {
    let published = visible_workspace_snapshot(snapshot);
    let body = serde_json::to_string_pretty(&published).expect("snapshot serializes");
    format!("{body}\n")
}

fn sync_display(status: SyncStatus, note: &str) -> String {
    match status {
        SyncStatus::NoUpstream => {
            if note == "no commits yet" {
                "❓ no commits yet".to_string()
            } else if note == "status failed" {
                "⚠️ status failed".to_string()
            } else {
                "❓ no upstream".to_string()
            }
        }
        SyncStatus::Behind => {
            let count = note
                .split("behind by ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("");
            if count.is_empty() {
                "⬇️ behind".to_string()
            } else {
                format!("⬇️ behind {count}")
            }
        }
        SyncStatus::Ahead => {
            let count = note
                .split("ahead by ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("");
            if count.is_empty() {
                "⬆️ ahead".to_string()
            } else {
                format!("⬆️ ahead {count}")
            }
        }
        SyncStatus::Diverged => {
            if let Some((ahead, rest)) = note.split_once("ahead ") {
                let _ = ahead;
                if let Some((a, behind)) = rest.split_once(", behind ") {
                    let b = behind.split_whitespace().next().unwrap_or("");
                    return format!("🔀 {a}/{b}");
                }
            }
            "🔀 diverged".to_string()
        }
        SyncStatus::UpToDate => "✅ current".to_string(),
    }
}

fn files_display(snapshot: &RepoSnapshot) -> String {
    let changed: BTreeSet<&str> = snapshot.changes.iter().map(|c| c.path.as_str()).collect();
    if snapshot.has_staged && snapshot.has_unstaged {
        "⚠️ staged+dirty".to_string()
    } else if snapshot.has_staged {
        "✨ staged".to_string()
    } else if snapshot.has_unstaged || snapshot.has_untracked {
        format!("📝 {} files", changed.len())
    } else {
        "💾 clean".to_string()
    }
}

fn to_verbose_row(s: &RepoSnapshot) -> VerboseRow {
    VerboseRow {
        repo: crate::helpers::format_checkout_repo_label(s),
        branch: crate::helpers::format_branch_with_merge(
            &format!("{} {}", get_branch_emoji(&s.branch), s.branch),
            s.merged_into_default,
        ),
        sync: sync_display(s.sync_status, &s.sync_note),
        files: files_display(s),
        note: String::new(),
    }
}

pub fn build_verbose_rows(
    snapshots: &[RepoSnapshot],
) -> (
    Vec<VerboseRow>,
    Vec<VerboseRow>,
    Vec<VerboseRow>,
    usize,
    usize,
) {
    let mut clean_default = Vec::new();
    let mut clean_non_default = Vec::new();
    let mut change_snaps = Vec::new();
    for s in snapshots {
        let has_changes = s.has_unstaged || s.has_staged || s.has_untracked;
        if has_changes {
            change_snaps.push(s);
        } else if is_default_branch(&s.branch, s.default_branch_override.as_deref()) {
            clean_default.push(s);
        } else {
            clean_non_default.push(s);
        }
    }
    clean_default.sort_by(|a, b| {
        get_sync_priority(a.sync_status)
            .cmp(&get_sync_priority(b.sync_status))
            .then_with(|| get_branch_priority(&a.branch).cmp(&get_branch_priority(&b.branch)))
            .then_with(|| compare_repo_paths_for_display(a, b))
    });
    clean_non_default.sort_by(|a, b| compare_repo_paths_for_display(a, b));
    change_snaps.sort_by(|a, b| compare_repo_paths_for_display(a, b));

    let clean_default: Vec<VerboseRow> = clean_default.into_iter().map(to_verbose_row).collect();
    let clean_non_default: Vec<VerboseRow> =
        clean_non_default.into_iter().map(to_verbose_row).collect();
    let change_repos: Vec<VerboseRow> = change_snaps.into_iter().map(to_verbose_row).collect();

    let mut repo_width = 20;
    let mut branch_width = 25;
    for r in clean_default
        .iter()
        .chain(&clean_non_default)
        .chain(&change_repos)
    {
        repo_width = repo_width.max(crate::helpers::visible_width(&r.repo));
        branch_width = branch_width.max(crate::helpers::visible_width(&r.branch));
    }
    (
        clean_default,
        clean_non_default,
        change_repos,
        repo_width,
        branch_width,
    )
}

pub fn build_summary_state(snapshots: &[RepoSnapshot]) -> SummaryState {
    let mut state = SummaryState::empty();
    for s in snapshots {
        if s.checkout_kind == CheckoutKind::Linked {
            state.linked_worktrees.insert(s.repo.clone());
        }
        if s.has_unstaged && s.has_staged {
            state.changes_both.insert(s.repo.clone());
        } else if s.has_unstaged {
            state.changes_uncommitted.insert(s.repo.clone());
        } else if s.has_staged {
            state.changes_staged.insert(s.repo.clone());
        }
        if s.has_untracked {
            state.changes_untracked.insert(s.repo.clone());
        }
        if is_attention_sync_note(&s.sync_note) {
            continue;
        }
        match s.sync_status {
            SyncStatus::Behind => {
                state.sync_behind.insert(s.repo.clone());
            }
            SyncStatus::Ahead => {
                state.sync_ahead.insert(s.repo.clone());
            }
            SyncStatus::Diverged => {
                state.sync_diverged.insert(s.repo.clone());
            }
            _ => {}
        }
        match get_branch_kind(&s.branch, s.default_branch_override.as_deref()) {
            BranchKind::Feature => {
                state.branch_feature.insert(s.repo.clone());
            }
            BranchKind::Bugfix => {
                state.branch_bugfix.insert(s.repo.clone());
            }
            BranchKind::Chore => {
                state.branch_chore.insert(s.repo.clone());
            }
            BranchKind::Release => {
                state.branch_release.insert(s.repo.clone());
            }
            BranchKind::Unknown => {
                state.branch_unknown.insert(s.repo.clone());
            }
            BranchKind::Default => {}
        }
    }
    state
}

pub fn non_default_branch_repos(summary: &SummaryState) -> Vec<String> {
    let mut all = Vec::new();
    all.extend(summary.branch_feature.iter().cloned());
    all.extend(summary.branch_bugfix.iter().cloned());
    all.extend(summary.branch_chore.iter().cloned());
    all.extend(summary.branch_release.iter().cloned());
    all.extend(summary.branch_unknown.iter().cloned());
    sorted_unique(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_repo(name: &str, ignored_change: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.to_string(),
            branch: "main".to_string(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: ignored_change,
            has_staged: false,
            has_untracked: false,
            changes: if ignored_change {
                vec![FileChange {
                    path: "README.md".to_string(),
                    staged_status: None,
                    unstaged_status: Some("M".to_string()),
                    untracked: false,
                    old_path: None,
                }]
            } else {
                vec![]
            },
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
        }
    }

    #[test]
    fn hidden_ignored_repo_stays_out_of_visible_snapshot() {
        let built = build_workspace_snapshot(
            &[sample_repo("app", true), sample_repo("notes", true)],
            &["notes".to_string()],
            false,
            &[],
        );
        let visible = visible_workspace_snapshot(&built);
        assert_eq!(
            visible
                .repos
                .iter()
                .map(|r| r.repo.as_str())
                .collect::<Vec<_>>(),
            vec!["app"]
        );
        assert_eq!(visible.ignored_repos, vec!["notes"]);
    }

    #[test]
    fn named_filter_includes_ignored_repo() {
        let built = build_workspace_snapshot(
            &[sample_repo("notes", true)],
            &["notes".to_string()],
            false,
            &["notes".to_string()],
        );
        let visible = visible_workspace_snapshot(&built);
        assert_eq!(visible.repos.len(), 1);
        assert!(visible.repos[0].ignored);
    }
}

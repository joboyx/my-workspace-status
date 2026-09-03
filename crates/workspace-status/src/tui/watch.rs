//! Live snapshot poll. `WS_STATUS_WATCH_MS=0` disables.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, UNIX_EPOCH};

use workspace_status_graph::{GraphModel, GraphRow};

use crate::snapshot::{WorkspaceRepoSnapshot, WorkspaceSnapshot};

use super::commit_files::CommitFileRow;
use super::drill::CommitFileSource;
use super::icons::status_letter_from_change;
use super::tree::{NodeKind, TreeNode, VisibleRow};

/// Default poll period.
pub const DEFAULT_WATCH_MS: u64 = 3000;
/// Faster than this spends more time in git than in the UI.
pub const MIN_WATCH_MS: u64 = 500;
/// How long a changed row stays highlighted.
pub const FLASH_MS: u64 = 800;
/// Repaint cadence while a flash is decaying (`FLASH_MS / 8`, floor 120).
pub const FLASH_TICK_MS: u64 = 120;

/// Poll period from `WS_STATUS_WATCH_MS`. `0` disables. Missing / invalid → default.
pub fn watch_interval_ms(raw: Option<&str>) -> u64 {
    let Some(raw) = raw else {
        return DEFAULT_WATCH_MS;
    };
    if raw.is_empty() {
        return DEFAULT_WATCH_MS;
    }
    let Ok(parsed) = raw.parse::<i64>() else {
        return DEFAULT_WATCH_MS;
    };
    if parsed < 0 {
        return DEFAULT_WATCH_MS;
    }
    if parsed == 0 {
        return 0;
    }
    (parsed as u64).max(MIN_WATCH_MS)
}

/// Remaining milliseconds until the next poll, from when this interval started.
///
/// A slow full-workspace collect must not push the next tick out by the
/// collect duration. `interval_ms == 0` disables (`u64::MAX`).
pub fn watch_remain_ms(interval_started: Instant, now: Instant, interval_ms: u64) -> u64 {
    if interval_ms == 0 {
        return u64::MAX;
    }
    interval_ms.saturating_sub(now.saturating_duration_since(interval_started).as_millis() as u64)
}

/// True when a live-watch poll is due. `interval_ms == 0` never fires.
pub fn watch_tick_due(interval_started: Instant, now: Instant, interval_ms: u64) -> bool {
    interval_ms > 0 && watch_remain_ms(interval_started, now, interval_ms) == 0
}

/// Watch identity for one checkout row.
///
/// Includes `HEAD` and `sync_note` so a new local commit on a clean branch
/// (or ahead 2→3 with the same [`crate::snapshot::SyncStatus`]) is a real
/// move. Dirty paths participate so an edit still flashes without `r`.
pub fn checkout_watch_identity(repo: &WorkspaceRepoSnapshot) -> String {
    let dirty: Vec<String> = repo
        .changes
        .iter()
        .map(|change| {
            format!(
                "{}:{}:{}:{}",
                change.path,
                change.staged_status.as_deref().unwrap_or(""),
                change.unstaged_status.as_deref().unwrap_or(""),
                change.untracked
            )
        })
        .collect();
    format!(
        "{}|{}|{}|{}|{}",
        repo.branch,
        repo.sync_status.as_str(),
        repo.sync_note,
        repo.head,
        dirty.join(";"),
    )
}

/// Checkout watch keys for a snapshot, keyed by repo path.
pub fn checkout_watch_identities(snapshot: &WorkspaceSnapshot) -> BTreeMap<String, String> {
    snapshot
        .repos
        .iter()
        .map(|repo| (repo.repo.clone(), checkout_watch_identity(repo)))
        .collect()
}

/// True when the right pane should reload after a watch apply.
///
/// Tree signatures cover file mtime / status. Checkout identities cover
/// `HEAD` / `sync_note` / dirty set so a chrome-silent commit still reloads
/// graph and status.
pub fn watch_needs_pane_reload(
    before_sigs: &BTreeMap<String, String>,
    after_sigs: &BTreeMap<String, String>,
    before_checkouts: &BTreeMap<String, String>,
    after_checkouts: &BTreeMap<String, String>,
) -> bool {
    before_sigs != after_sigs || before_checkouts != after_checkouts
}

/// `changeSignatures` disk token: `size:mtimeMs`, or `gone` when missing.
fn file_disk_token(cwd: &Path, repo: &str, rel: &str) -> String {
    let abs = cwd.join(repo).join(rel);
    match fs::metadata(&abs) {
        Ok(meta) => {
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!("{}:{mtime_ms}", meta.len())
        }
        Err(_) => "gone".into(),
    }
}

/// Semantic signature for one painted row.
///
/// File rows match `changeSignatures`: status letter plus `size:mtimeMs`
/// (or `gone`). An in-place save of an already-modified file therefore flashes.
/// Chrome rows use path / branch / sync enum / sync note / HEAD / change
/// count / fold so a new commit or ahead 2→3 is a semantic update, while
/// glyph paint still is not.
pub fn row_signature(row: &VisibleRow, cwd: &Path) -> String {
    match row.kind {
        NodeKind::File => {
            let status = row
                .file
                .as_ref()
                .map(status_letter_from_change)
                .unwrap_or(super::icons::FileStatusLetter::M);
            let disk = match (row.repo.as_deref(), row.file.as_ref()) {
                (Some(repo), Some(file)) => file_disk_token(cwd, repo, &file.path),
                _ => "gone".into(),
            };
            format!("{}:{disk}", status.as_str())
        }
        _ => format!(
            "chrome:{}:{}:{}:{}:{}:{}:{}",
            row.chrome.path,
            row.chrome.branch,
            row.chrome.sync_status.map(|s| s.as_str()).unwrap_or(""),
            row.chrome.sync_note,
            row.chrome.head,
            row.chrome.change_count,
            row.folded
        ),
    }
}

/// Signatures keyed by row id from a full (unfolded) walk so folded files
/// still participate in change detection.
pub fn tree_signatures(tree: &TreeNode, cwd: &Path) -> BTreeMap<String, String> {
    let rows = super::tree::flatten(tree, &std::collections::HashSet::new());
    rows.into_iter()
        .map(|row| {
            let sig = row_signature(&row, cwd);
            (row.id, sig)
        })
        .collect()
}

/// Why a row is flashing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashKind {
    /// Id present in `after`, not in `before`.
    Add,
    /// Id in both maps with a different signature.
    Update,
    /// Id present in `before`, not in `after`.
    Remove,
}

/// One decaying flash stamp.
#[derive(Clone, Copy, Debug)]
pub struct FlashStamp {
    /// When the flash was stamped.
    pub at: Instant,
    /// Add, update, or remove.
    pub kind: FlashKind,
}

/// Classify one id against a before/after signature pair.
///
/// Unchanged ids (same signature in both maps) return `None`.
pub fn classify_flash(
    id: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Option<FlashKind> {
    match (before.get(id), after.get(id)) {
        (None, Some(_)) => Some(FlashKind::Add),
        (Some(prev), Some(next)) if prev != next => Some(FlashKind::Update),
        (Some(_), None) => Some(FlashKind::Remove),
        _ => None,
    }
}

/// Ids whose signature appeared or changed. Removals are included.
/// The whole tree is not treated as one change.
pub fn changed_row_ids(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    flashable_row_ids(before, after, true)
}

/// True when `after` shares no keys with `before` (or `before` is empty).
///
/// A repo switch, first paint, or wholly different commit set is a new list,
/// not a set of added/removed rows — do not flash.
pub fn is_new_row_set(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> bool {
    before.is_empty() || after.keys().all(|id| !before.contains_key(id))
}

/// Ids that should flash for this signature diff, with kind.
///
/// `include_adds` is false for graph autoload (older commits appended) so a
/// longer window does not flash every newly loaded row.
pub fn flashable_row_kinds(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    include_adds: bool,
) -> Vec<(String, FlashKind)> {
    let mut out = Vec::new();
    for id in after.keys().chain(before.keys()) {
        let Some(kind) = classify_flash(id, before, after) else {
            continue;
        };
        if kind == FlashKind::Add && !include_adds {
            continue;
        }
        out.push((id.clone(), kind));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup();
    out
}

/// Ids that should flash for this signature diff.
///
/// `include_adds` is false for graph autoload (older commits appended) so a
/// longer window does not flash every newly loaded row.
pub fn flashable_row_ids(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    include_adds: bool,
) -> Vec<String> {
    flashable_row_kinds(before, after, include_adds)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// Linear 1 → 0 over [`FLASH_MS`].
pub fn flash_strength(elapsed: Duration) -> f32 {
    let ms = elapsed.as_millis() as f32;
    let window = FLASH_MS as f32;
    if ms <= 0.0 {
        1.0
    } else if ms >= window {
        0.0
    } else {
        1.0 - ms / window
    }
}

/// True when `elapsed` is still inside the flash window.
pub fn flash_active(elapsed: Duration) -> bool {
    flash_strength(elapsed) > 0.0
}

/// Drop flash stamps that have finished decaying.
pub fn prune_flashes(flashes: &mut HashMap<String, FlashStamp>, now: Instant) {
    flashes.retain(|_, stamp| flash_active(now.saturating_duration_since(stamp.at)));
}

/// A removed row kept in place for [`FLASH_MS`] so the flash is visible.
#[derive(Clone, Debug)]
pub struct GhostRow<T> {
    /// Stable row id (same as the live list).
    pub id: String,
    /// Last painted row.
    pub row: T,
    /// When the removal was stamped.
    pub flashed_at: Instant,
    /// Index in the last live list.
    pub index: usize,
}

/// Drop ghosts whose flash has expired.
pub fn prune_ghosts<T>(ghosts: &mut Vec<GhostRow<T>>, now: Instant) {
    ghosts.retain(|g| flash_active(now.saturating_duration_since(g.flashed_at)));
}

/// Capture live rows that disappeared from `after`.
///
/// `id_of` is the painted-row identity used to merge ghosts. `sig_id_of` is
/// the signature-map key (tree ids are the same; commit-file ids include
/// repo and source).
pub fn capture_removal_ghosts<T: Clone>(
    old_rows: &[T],
    id_of: impl Fn(&T) -> &str,
    sig_id_of: impl Fn(&T) -> String,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    now: Instant,
) -> Vec<GhostRow<T>> {
    let mut out = Vec::new();
    for (index, row) in old_rows.iter().enumerate() {
        let sig_id = sig_id_of(row);
        if before.contains_key(&sig_id) && !after.contains_key(&sig_id) {
            out.push(GhostRow {
                id: id_of(row).to_string(),
                row: row.clone(),
                flashed_at: now,
                index,
            });
        }
    }
    out
}

/// Re-insert active ghosts at their last live index. Live ids win.
pub fn merge_ghost_rows<T: Clone>(
    live: &[T],
    ghosts: &[GhostRow<T>],
    id_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    let live_ids: std::collections::HashSet<&str> = live.iter().map(&id_of).collect();
    let mut out = live.to_vec();
    let mut extras: Vec<&GhostRow<T>> = ghosts
        .iter()
        .filter(|g| !live_ids.contains(g.id.as_str()))
        .collect();
    extras.sort_by_key(|g| g.index);
    for ghost in extras {
        let at = ghost.index.min(out.len());
        out.insert(at, ghost.row.clone());
    }
    out
}

/// Graph window identity used to distinguish autoload from a repo switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphFlashMeta {
    /// Checkout path the model was loaded for.
    pub repo: String,
    /// `git log --skip` of this window.
    pub skip: usize,
    /// `git log --max-count` of this window.
    pub limit: usize,
    /// Log-prefix commit ids (newest first), excluding extra stash parents.
    pub commit_ids: Vec<String>,
}

/// How to apply a newly loaded graph against the previous signature map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphFlashDecision {
    /// Focused checkout disagrees with the painted model — ignore this load.
    Stale,
    /// First paint, repo switch, or a disjoint identity set: seed, do not flash.
    Seed,
    /// Same list: flash overlapping adds/updates/removes. Autoload skips pure adds.
    Apply {
        /// When false, newly appeared ids (older autoload pages) do not flash.
        include_adds: bool,
    },
}

/// Stable id for one graph row (`{repo}#{kind}:{id}`).
pub fn graph_row_id(row: &GraphRow) -> String {
    match row {
        GraphRow::Uncommitted { .. } => "uncommitted".into(),
        GraphRow::Stash(stash) => format!("stash:{}", stash.stash_ref),
        GraphRow::Commit { commit, .. } => format!("commit:{}", commit.id),
        GraphRow::Worktree(wt) => format!("worktree:{}", wt.path),
    }
}

/// Same sha / `stash@{n}` / uncommitted row in another repo is a different row.
pub fn graph_row_identity(repo: &str, row: &GraphRow) -> String {
    format!("{repo}#{}", graph_row_id(row))
}

/// Model signature for one graph row. Never painted segments.
pub fn graph_row_signature(row: &GraphRow) -> String {
    match row {
        GraphRow::Uncommitted { has_changes } => format!("uncommitted:{has_changes}"),
        GraphRow::Stash(stash) => format!(
            "stash:{}|{}",
            stash.subject,
            stash.parent_id.as_deref().unwrap_or("")
        ),
        GraphRow::Commit {
            commit,
            is_head,
            worktrees,
        } => {
            let mut refs: Vec<&str> = commit.refs.iter().map(|r| r.name.as_str()).collect();
            refs.sort_unstable();
            let mut wt_paths: Vec<&str> = worktrees.iter().map(|w| w.path.as_str()).collect();
            wt_paths.sort_unstable();
            format!(
                "commit:{}|{}|{}|{}",
                commit.subject,
                refs.join(","),
                is_head,
                wt_paths.join(",")
            )
        }
        GraphRow::Worktree(wt) => format!(
            "worktree:{}|{}|{}",
            wt.branch.as_deref().unwrap_or(""),
            wt.head_id.as_deref().unwrap_or(""),
            wt.ignored
        ),
    }
}

/// Signatures for every visible graph row, keyed by [`graph_row_identity`].
pub fn graph_row_signatures(model: &GraphModel, repo: &str) -> BTreeMap<String, String> {
    model
        .visible_rows()
        .iter()
        .map(|row| (graph_row_identity(repo, row), graph_row_signature(row)))
        .collect()
}

/// Log-window meta for autoload / repo-switch detection.
pub fn graph_flash_meta(model: &GraphModel, repo: &str) -> GraphFlashMeta {
    let n = model.window_count().min(model.commits.len());
    GraphFlashMeta {
        repo: repo.to_string(),
        skip: model.skip,
        limit: model.limit,
        commit_ids: model.commits[..n].iter().map(|c| c.id.clone()).collect(),
    }
}

fn is_autoload_prefix(prev: &GraphFlashMeta, next: &GraphFlashMeta) -> bool {
    prev.skip == next.skip
        && next.commit_ids.len() > prev.commit_ids.len()
        && next.commit_ids.starts_with(&prev.commit_ids)
}

/// Decide whether a newly loaded graph should flash, seed, or be ignored.
pub fn graph_flash_decision(
    focused_repo: Option<&str>,
    painted_repo: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    prev_meta: Option<&GraphFlashMeta>,
    next_meta: &GraphFlashMeta,
) -> GraphFlashDecision {
    if let Some(focused) = focused_repo {
        if focused != painted_repo {
            return GraphFlashDecision::Stale;
        }
    }
    if before.is_empty() || is_new_row_set(before, after) {
        return GraphFlashDecision::Seed;
    }
    if let Some(prev) = prev_meta {
        if prev.repo != next_meta.repo {
            return GraphFlashDecision::Seed;
        }
        if is_autoload_prefix(prev, next_meta) {
            return GraphFlashDecision::Apply {
                include_adds: false,
            };
        }
    }
    GraphFlashDecision::Apply { include_adds: true }
}

fn commit_file_source_key(source: &CommitFileSource) -> String {
    match source {
        CommitFileSource::Commit { commit_id } => format!("commit:{commit_id}"),
        CommitFileSource::Stash { stash_ref } => format!("stash:{stash_ref}"),
        CommitFileSource::Worktree => "worktree".into(),
    }
}

/// Same path in another commit / repo is a different row.
pub fn commit_file_identity(repo: &str, source: &CommitFileSource, row_id: &str) -> String {
    format!("{repo}#{}#{row_id}", commit_file_source_key(source))
}

/// Path + status (+ old path). Dirs use `dir|{path}`.
pub fn commit_file_signature(row: &CommitFileRow) -> String {
    match row.file.as_ref() {
        Some(file) => format!(
            "{}|{}|{}",
            file.path,
            file.status,
            file.old_path.as_deref().unwrap_or("")
        ),
        None => format!("dir|{}", row.path),
    }
}

/// Signatures for a flattened commit-file list.
pub fn commit_file_signatures(
    repo: &str,
    source: &CommitFileSource,
    rows: &[CommitFileRow],
) -> BTreeMap<String, String> {
    rows.iter()
        .map(|row| {
            (
                commit_file_identity(repo, source, &row.id),
                commit_file_signature(row),
            )
        })
        .collect()
}

/// Tree node ids for a checkout path (`repo:` and/or `checkout:`).
pub fn checkout_flash_ids(path: &str) -> Vec<String> {
    vec![format!("repo:{path}"), format!("checkout:{path}")]
}

#[cfg(test)]
mod tests {
    use super::super::tree::NodeChrome;
    use super::*;
    use crate::snapshot::{CheckoutKind, FileChange, SyncStatus, WorkspaceRepoSnapshot};
    use std::path::PathBuf;
    use workspace_status_graph::{Commit, GraphRef, Stash, Worktree};

    fn modified_file_row(repo: &str, path: &str) -> VisibleRow {
        VisibleRow {
            id: format!("file:{repo}:{path}"),
            depth: 2,
            kind: NodeKind::File,
            label: format!("M {path}"),
            repo: Some(repo.into()),
            file: Some(FileChange {
                path: path.into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }),
            ..VisibleRow::default()
        }
    }

    fn tmp_workspace(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mini_model(ids: &[&str]) -> GraphModel {
        GraphModel {
            uncommitted: Some(false),
            commits: ids
                .iter()
                .map(|id| Commit {
                    id: (*id).into(),
                    subject: format!("s-{id}"),
                    ..Commit::default()
                })
                .collect(),
            window: ids.len(),
            skip: 0,
            limit: 300,
            ..GraphModel::default()
        }
    }

    #[test]
    fn zero_disables() {
        assert_eq!(watch_interval_ms(Some("0")), 0);
    }

    #[test]
    fn default_and_clamp() {
        assert_eq!(watch_interval_ms(None), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("")), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("-1")), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("abc")), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("100")), MIN_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("5000")), 5000);
    }

    fn chrome_row(
        path: &str,
        branch: &str,
        status: SyncStatus,
        note: &str,
        head: &str,
    ) -> VisibleRow {
        VisibleRow {
            id: format!("repo:{path}"),
            kind: NodeKind::Repo,
            chrome: NodeChrome {
                path: path.into(),
                branch: branch.into(),
                sync_status: Some(status),
                sync_note: note.into(),
                head: head.into(),
                change_count: 0,
                ..NodeChrome::default()
            },
            ..VisibleRow::default()
        }
    }

    fn checkout_row(
        path: &str,
        branch: &str,
        status: SyncStatus,
        note: &str,
        head: &str,
        dirty: &[(&str, &str)],
    ) -> WorkspaceRepoSnapshot {
        WorkspaceRepoSnapshot {
            repo: path.into(),
            ignored: false,
            branch: branch.into(),
            sync_status: status,
            sync_note: note.into(),
            head: head.into(),
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
            has_unstaged: !dirty.is_empty(),
            has_staged: false,
            has_untracked: false,
            changes: dirty
                .iter()
                .map(|(file, letter)| FileChange {
                    path: (*file).into(),
                    staged_status: None,
                    unstaged_status: Some((*letter).into()),
                    untracked: false,
                    old_path: None,
                })
                .collect(),
        }
    }

    #[test]
    fn next_tick_is_from_interval_start_not_collect_end() {
        let started = Instant::now();
        let after_collect = started + Duration::from_millis(2000);
        assert_eq!(watch_remain_ms(started, after_collect, 3000), 1000);
        assert!(!watch_tick_due(started, after_collect, 3000));
        assert!(watch_tick_due(
            started,
            started + Duration::from_millis(3000),
            3000
        ));
        assert_eq!(
            watch_remain_ms(started, started + Duration::from_millis(2000), 0),
            u64::MAX
        );
        // Stamping after collect would leave a full interval — that is the bug.
        let stamped_after_collect = after_collect;
        assert_eq!(
            watch_remain_ms(stamped_after_collect, after_collect, 3000),
            3000
        );
    }

    #[test]
    fn chrome_signature_moves_on_head_or_sync_note() {
        let cwd = Path::new("/nonexistent");
        let base = chrome_row("alpha", "feature/watch", SyncStatus::NoUpstream, "", "aaa");
        let same_chrome = chrome_row("alpha", "feature/watch", SyncStatus::NoUpstream, "", "aaa");
        assert_eq!(row_signature(&base, cwd), row_signature(&same_chrome, cwd));

        let new_head = chrome_row("alpha", "feature/watch", SyncStatus::NoUpstream, "", "bbb");
        assert_ne!(
            row_signature(&base, cwd),
            row_signature(&new_head, cwd),
            "HEAD-only commit must change chrome watch identity"
        );

        let ahead_two = chrome_row(
            "alpha",
            "feature/watch",
            SyncStatus::Ahead,
            "ahead by 2 commits",
            "aaa",
        );
        let ahead_three = chrome_row(
            "alpha",
            "feature/watch",
            SyncStatus::Ahead,
            "ahead by 3 commits",
            "ccc",
        );
        assert_ne!(
            row_signature(&ahead_two, cwd),
            row_signature(&ahead_three, cwd),
            "ahead 2→3 with the same SyncStatus must change chrome watch identity"
        );
    }

    #[test]
    fn checkout_identity_and_pane_reload_see_silent_head_and_dirty() {
        let before_row = checkout_row(
            "alpha",
            "feature/watch",
            SyncStatus::NoUpstream,
            "",
            "aaa",
            &[],
        );
        let after_head = checkout_row(
            "alpha",
            "feature/watch",
            SyncStatus::NoUpstream,
            "",
            "bbb",
            &[],
        );
        assert_ne!(
            checkout_watch_identity(&before_row),
            checkout_watch_identity(&after_head)
        );

        let ahead_two = checkout_row(
            "alpha",
            "feature/watch",
            SyncStatus::Ahead,
            "ahead by 2 commits",
            "aaa",
            &[],
        );
        let ahead_three = checkout_row(
            "alpha",
            "feature/watch",
            SyncStatus::Ahead,
            "ahead by 3 commits",
            "aaa",
            &[],
        );
        assert_ne!(
            checkout_watch_identity(&ahead_two),
            checkout_watch_identity(&ahead_three)
        );

        let mut before_sigs = BTreeMap::new();
        before_sigs.insert(
            "repo:alpha".into(),
            row_signature(
                &chrome_row("alpha", "feature/watch", SyncStatus::NoUpstream, "", "aaa"),
                Path::new("/nonexistent"),
            ),
        );
        let mut after_sigs = before_sigs.clone();
        after_sigs.insert(
            "repo:alpha".into(),
            row_signature(
                &chrome_row("alpha", "feature/watch", SyncStatus::NoUpstream, "", "bbb"),
                Path::new("/nonexistent"),
            ),
        );
        let mut before_checkouts = BTreeMap::new();
        before_checkouts.insert("alpha".into(), checkout_watch_identity(&before_row));
        let mut after_checkouts = BTreeMap::new();
        after_checkouts.insert("alpha".into(), checkout_watch_identity(&after_head));
        assert!(watch_needs_pane_reload(
            &before_sigs,
            &after_sigs,
            &before_checkouts,
            &after_checkouts
        ));
        assert!(!watch_needs_pane_reload(
            &before_sigs,
            &before_sigs,
            &before_checkouts,
            &before_checkouts
        ));

        let dirty = checkout_row(
            "beta",
            "feature/other",
            SyncStatus::NoUpstream,
            "",
            "ddd",
            &[("README.md", "M")],
        );
        assert_ne!(
            checkout_watch_identity(&checkout_row(
                "beta",
                "feature/other",
                SyncStatus::NoUpstream,
                "",
                "ddd",
                &[]
            )),
            checkout_watch_identity(&dirty)
        );
    }

    #[test]
    fn only_changed_ids_flash() {
        let mut before = BTreeMap::new();
        before.insert("file:app:a".into(), "M:12:1".into());
        before.insert("file:app:b".into(), "M:12:1".into());
        before.insert("repo:app".into(), "chrome:app:false:app".into());
        let mut after = before.clone();
        after.insert("file:app:a".into(), "S:12:1".into());
        let changed = changed_row_ids(&before, &after);
        assert_eq!(changed, vec!["file:app:a".to_string()]);
    }

    #[test]
    fn identical_maps_flash_nothing() {
        let mut map = BTreeMap::new();
        map.insert("workspace".into(), "chrome:ws:false:".into());
        assert!(changed_row_ids(&map, &map).is_empty());
    }

    #[test]
    fn in_place_save_of_already_modified_file_changes_signature() {
        let cwd = tmp_workspace("ws-watch-sig");
        let rel = Path::new("demo/src");
        fs::create_dir_all(cwd.join(rel)).unwrap();
        let file = cwd.join("demo/src/main.ts");
        fs::write(&file, "a\n").unwrap();
        let row = modified_file_row("demo", "src/main.ts");

        let first = row_signature(&row, &cwd);
        assert!(first.starts_with("M:"), "{first}");
        assert!(!first.ends_with(":gone"), "{first}");

        let again = row_signature(&row, &cwd);
        assert_eq!(first, again);

        fs::write(&file, "a\nb\n").unwrap();
        let after_save = row_signature(&row, &cwd);
        assert_ne!(
            first, after_save,
            "same-letter in-place save must change the watch signature"
        );
        assert!(after_save.starts_with("M:"), "{after_save}");

        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn missing_worktree_file_signs_gone() {
        let row = modified_file_row("demo", "src/gone.ts");
        let sig = row_signature(&row, Path::new("/nonexistent"));
        assert_eq!(sig, "M:gone");
    }

    #[test]
    fn disjoint_row_set_is_new() {
        let mut before = BTreeMap::new();
        before.insert("demo#commit:aaa".into(), "s".into());
        let mut after = BTreeMap::new();
        after.insert("notes#commit:aaa".into(), "s".into());
        assert!(is_new_row_set(&before, &after));
        let both = flashable_row_ids(&before, &after, true);
        assert!(both.contains(&"demo#commit:aaa".to_string()));
        assert!(both.contains(&"notes#commit:aaa".to_string()));
        // Callers must check is_new_row_set before flashing those ids.
    }

    #[test]
    fn overlapping_set_is_not_new() {
        let mut before = BTreeMap::new();
        before.insert("demo#commit:aaa".into(), "s".into());
        before.insert("demo#commit:bbb".into(), "s".into());
        let mut after = before.clone();
        after.insert("demo#commit:ccc".into(), "s".into());
        assert!(!is_new_row_set(&before, &after));
    }

    #[test]
    fn autoload_skips_new_ids_but_keeps_updates_and_removes() {
        let mut before = BTreeMap::new();
        before.insert("demo#commit:aaa".into(), "s-aaa".into());
        before.insert("demo#commit:bbb".into(), "s-bbb".into());
        let mut after = before.clone();
        after.insert("demo#commit:ccc".into(), "s-ccc".into());
        after.insert("demo#commit:aaa".into(), "s-aaa-updated".into());
        after.remove("demo#commit:bbb");
        let flashed = flashable_row_ids(&before, &after, false);
        assert!(flashed.contains(&"demo#commit:aaa".to_string()));
        assert!(flashed.contains(&"demo#commit:bbb".to_string()));
        assert!(!flashed.contains(&"demo#commit:ccc".to_string()));
    }

    #[test]
    fn classifies_add_update_remove_from_signature_maps() {
        let mut before = BTreeMap::new();
        before.insert("file:app:a".into(), "M:1".into());
        before.insert("file:app:b".into(), "M:1".into());
        before.insert("file:app:c".into(), "M:1".into());
        let mut after = before.clone();
        after.insert("file:app:a".into(), "M:2".into());
        after.insert("file:app:d".into(), "A:1".into());
        after.remove("file:app:c");

        assert_eq!(
            classify_flash("file:app:d", &before, &after),
            Some(FlashKind::Add)
        );
        assert_eq!(
            classify_flash("file:app:a", &before, &after),
            Some(FlashKind::Update)
        );
        assert_eq!(
            classify_flash("file:app:c", &before, &after),
            Some(FlashKind::Remove)
        );
        assert_eq!(classify_flash("file:app:b", &before, &after), None);

        let kinds = flashable_row_kinds(&before, &after, true);
        assert_eq!(
            kinds
                .iter()
                .find(|(id, _)| id == "file:app:d")
                .map(|(_, kind)| *kind),
            Some(FlashKind::Add)
        );
        assert_eq!(
            kinds
                .iter()
                .find(|(id, _)| id == "file:app:a")
                .map(|(_, kind)| *kind),
            Some(FlashKind::Update)
        );
        assert_eq!(
            kinds
                .iter()
                .find(|(id, _)| id == "file:app:c")
                .map(|(_, kind)| *kind),
            Some(FlashKind::Remove)
        );
        assert!(kinds.iter().all(|(id, _)| id != "file:app:b"));

        let no_adds = flashable_row_kinds(&before, &after, false);
        assert!(no_adds.iter().all(|(id, _)| id != "file:app:d"));
        assert!(no_adds
            .iter()
            .any(|(id, kind)| id == "file:app:a" && *kind == FlashKind::Update));
        assert!(no_adds
            .iter()
            .any(|(id, kind)| id == "file:app:c" && *kind == FlashKind::Remove));
    }

    #[test]
    fn same_sha_different_repo_is_different_identity() {
        let row = GraphRow::Commit {
            commit: Commit {
                id: "aaa".into(),
                subject: "s".into(),
                refs: vec![GraphRef::local("main")],
                ..Commit::default()
            },
            is_head: true,
            worktrees: Vec::new(),
        };
        assert_ne!(
            graph_row_identity("demo", &row),
            graph_row_identity("notes", &row)
        );
    }

    #[test]
    fn graph_flash_decision_seeds_on_repo_switch() {
        let a = mini_model(&["aaa"]);
        let b = mini_model(&["aaa"]);
        let before = graph_row_signatures(&a, "demo");
        let after = graph_row_signatures(&b, "notes");
        let prev = graph_flash_meta(&a, "demo");
        let next = graph_flash_meta(&b, "notes");
        assert_eq!(
            graph_flash_decision(Some("notes"), "notes", &before, &after, Some(&prev), &next),
            GraphFlashDecision::Seed
        );
    }

    #[test]
    fn graph_flash_decision_skips_adds_on_autoload_prefix() {
        let prev_model = mini_model(&["aaa", "bbb"]);
        let next_model = mini_model(&["aaa", "bbb", "ccc"]);
        let before = graph_row_signatures(&prev_model, "demo");
        let after = graph_row_signatures(&next_model, "demo");
        let prev = graph_flash_meta(&prev_model, "demo");
        let next = graph_flash_meta(&next_model, "demo");
        assert_eq!(
            graph_flash_decision(Some("demo"), "demo", &before, &after, Some(&prev), &next),
            GraphFlashDecision::Apply {
                include_adds: false
            }
        );
    }

    #[test]
    fn graph_flash_decision_stale_when_focus_disagrees() {
        let model = mini_model(&["aaa"]);
        let sigs = graph_row_signatures(&model, "demo");
        let meta = graph_flash_meta(&model, "demo");
        assert_eq!(
            graph_flash_decision(Some("notes"), "demo", &sigs, &sigs, Some(&meta), &meta),
            GraphFlashDecision::Stale
        );
    }

    #[test]
    fn flash_strength_ramps_to_zero() {
        assert_eq!(flash_strength(Duration::from_millis(0)), 1.0);
        assert!(flash_strength(Duration::from_millis(400)) > 0.4);
        assert!(flash_strength(Duration::from_millis(400)) < 0.6);
        assert_eq!(flash_strength(Duration::from_millis(FLASH_MS)), 0.0);
        assert!(!flash_active(Duration::from_millis(FLASH_MS)));
        assert!(flash_active(Duration::from_millis(0)));
    }

    #[test]
    fn merge_ghosts_reinserts_at_index() {
        let live = vec!["a".to_string(), "c".to_string()];
        let ghosts = vec![GhostRow {
            id: "b".into(),
            row: "b".to_string(),
            flashed_at: Instant::now(),
            index: 1,
        }];
        let merged = merge_ghost_rows(&live, &ghosts, |s| s.as_str());
        assert_eq!(merged, vec!["a", "b", "c"]);
    }

    #[test]
    fn graph_signature_ignores_ref_paint_order() {
        let mut left = GraphRow::Commit {
            commit: Commit {
                id: "aaa".into(),
                subject: "s".into(),
                refs: vec![GraphRef::local("main"), GraphRef::local("topic")],
                ..Commit::default()
            },
            is_head: false,
            worktrees: Vec::new(),
        };
        let mut right = left.clone();
        if let GraphRow::Commit { commit, .. } = &mut right {
            commit.refs.reverse();
        }
        assert_eq!(graph_row_signature(&left), graph_row_signature(&right));
        if let GraphRow::Commit { commit, .. } = &mut left {
            commit.subject = "other".into();
        }
        assert_ne!(graph_row_signature(&left), graph_row_signature(&right));
        let stash = GraphRow::Stash(Stash {
            stash_ref: "stash@{0}".into(),
            subject: "WIP".into(),
            parent_id: Some("aaa".into()),
            ..Stash::default()
        });
        let wt = GraphRow::Worktree(Worktree {
            path: "wt".into(),
            head_id: Some("aaa".into()),
            branch: Some("feature".into()),
            ignored: false,
            is_current: false,
        });
        assert!(graph_row_signature(&stash).starts_with("stash:"));
        assert!(graph_row_signature(&wt).starts_with("worktree:"));
    }
}

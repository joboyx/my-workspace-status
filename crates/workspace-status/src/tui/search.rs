//! Vim-style `/` search on the focused pane.
//!
//! Tree matches include folded rows. Focusing a tree match unfolds its
//! ancestors so the row is visible. Hidden ignored repos stay out of tree
//! search unless shown (`.` / `-a`). Graph search matches subject, author,
//! painted (relative or local) or UTC time, branch and tag names, and sha. Commit-file search
//! matches paths. Diff search matches painted line text.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use workspace_status_graph::{format_relative_date, format_utc_timestamp, short_id, GraphRow};

use super::drill::CommitFile;
use super::tree::{flatten, TreeNode};
use crate::helpers::visible_width;

/// Pane `/` binds at search start. `n`/`N` stay on this pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchPane {
    #[default]
    Tree,
    Graph,
    CommitFiles,
    Diff,
}

/// Case-insensitive substring matches on `label`. Empty query → no hits.
pub fn match_indices(labels: &[&str], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    labels
        .iter()
        .enumerate()
        .filter(|(_, label)| label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// Next/prev match id with wrap. Empty `ids` → `None`.
pub fn step_match_id(ids: &[String], current_id: Option<&str>, dir: i32) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let pos = current_id.and_then(|id| ids.iter().position(|x| x == id));
    let Some(pos) = pos else {
        return if dir < 0 {
            ids.last().cloned()
        } else {
            ids.first().cloned()
        };
    };
    let len = ids.len() as i32;
    let next = (pos as i32 + dir).rem_euclid(len) as usize;
    ids.get(next).cloned()
}

/// Stable ids whose labels match `query`, in tree order (including folded).
pub fn collect_match_ids(tree: &TreeNode, query: &str) -> Vec<String> {
    let all = flatten(tree, &HashSet::new());
    let labels: Vec<&str> = all.iter().map(|r| r.label.as_str()).collect();
    match_indices(&labels, query)
        .into_iter()
        .map(|i| all[i].id.clone())
        .collect()
}

/// Path from the root to `target` (inclusive). Empty when missing.
pub fn path_to(tree: &TreeNode, target: &str) -> Vec<String> {
    let mut path = Vec::new();
    if find_path(tree, target, &mut path) {
        path
    } else {
        Vec::new()
    }
}

fn find_path(node: &TreeNode, target: &str, path: &mut Vec<String>) -> bool {
    path.push(node.id.clone());
    if node.id == target {
        return true;
    }
    for child in &node.children {
        if find_path(child, target, path) {
            return true;
        }
    }
    path.pop();
    false
}

/// Unfold every ancestor of `focus_id` so the row can paint.
pub fn unfold_ancestors(
    tree: &TreeNode,
    folds: &HashSet<String>,
    focus_id: &str,
) -> HashSet<String> {
    let mut next = folds.clone();
    for id in path_to(tree, focus_id) {
        next.remove(&id);
    }
    next
}

/// Focus a match. `dir` is `0` (first), `1` (next), or `-1` (previous).
/// Unfolds ancestors of the chosen match only.
pub fn focus_tree_search(
    tree: &TreeNode,
    folds: &HashSet<String>,
    query: &str,
    current_id: Option<&str>,
    dir: i32,
) -> (HashSet<String>, Option<String>) {
    let ids = collect_match_ids(tree, query);
    let focus_id = if dir == 0 {
        ids.first().cloned()
    } else {
        step_match_id(&ids, current_id, dir)
    };
    let Some(focus_id) = focus_id else {
        return (folds.clone(), None);
    };
    (unfold_ancestors(tree, folds, &focus_id), Some(focus_id))
}

/// Search text for one graph row.
///
/// Commits match subject, author, painted relative date, painted local
/// timestamp (`YYYY-MM-DD HH:MM`), UTC timestamp (`YYYY-MM-DD HH:MM`),
/// branch and tag names, and full plus short sha.
/// Stash and worktree rows use the same fields when the model has them.
/// Uncommitted matches the word `uncommitted`. Dates use the same 3-hour
/// relative / older-local formatter as the painted spacer, plus a stable
/// UTC form so a calendar date still hits when the row shows a relative age.
pub fn graph_row_search_text(row: &GraphRow) -> String {
    graph_row_search_text_at(row, search_now_unix())
}

fn search_now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn graph_row_search_text_at(row: &GraphRow, now_unix: i64) -> String {
    match row {
        GraphRow::Uncommitted { .. } => "uncommitted".into(),
        GraphRow::Stash(stash) => {
            let mut parts = Vec::new();
            push_part(&mut parts, &stash.subject);
            push_part(&mut parts, &stash.stash_ref);
            push_part(&mut parts, &stash.author_name);
            push_sha(&mut parts, &stash.id);
            push_dates(&mut parts, stash.author_date_unix, now_unix);
            parts.join(" ")
        }
        GraphRow::Worktree(wt) => {
            let mut parts = Vec::new();
            push_part(&mut parts, &wt.path);
            if let Some(branch) = &wt.branch {
                push_part(&mut parts, branch);
            }
            if let Some(id) = &wt.head_id {
                push_sha(&mut parts, id);
            }
            parts.join(" ")
        }
        GraphRow::Commit { commit, .. } => {
            let mut parts = Vec::new();
            push_part(&mut parts, &commit.subject);
            for graph_ref in &commit.refs {
                push_part(&mut parts, &graph_ref.name);
            }
            push_part(&mut parts, &commit.author_name);
            push_sha(&mut parts, &commit.id);
            push_dates(&mut parts, commit.author_date_unix, now_unix);
            parts.join(" ")
        }
    }
}

fn push_part(parts: &mut Vec<String>, value: &str) {
    if !value.is_empty() {
        parts.push(value.to_string());
    }
}

fn push_sha(parts: &mut Vec<String>, id: &str) {
    push_part(parts, id);
    let short = short_id(id);
    if short != id {
        push_part(parts, short);
    }
}

fn push_dates(parts: &mut Vec<String>, unix: i64, now_unix: i64) {
    if unix <= 0 {
        return;
    }
    let painted = format_relative_date(unix, now_unix);
    push_part(parts, &painted);
    let utc = format_utc_timestamp(unix);
    if utc != painted {
        push_part(parts, &utc);
    }
}

/// Indices of graph rows whose search text matches `query`.
pub fn collect_graph_match_indices(rows: &[GraphRow], query: &str) -> Vec<usize> {
    let labels: Vec<String> = rows.iter().map(graph_row_search_text).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    match_indices(&refs, query)
}

/// Next/prev graph match index with wrap. `dir` 0 = first hit.
pub fn focus_graph_search(
    rows: &[GraphRow],
    query: &str,
    current: usize,
    dir: i32,
) -> Option<usize> {
    let hits = collect_graph_match_indices(rows, query);
    if hits.is_empty() {
        return None;
    }
    if dir == 0 {
        return hits.first().copied();
    }
    step_match_index(&hits, current, dir)
}

/// Indices of commit-file paths that match `query`.
pub fn collect_commit_file_match_indices(files: &[CommitFile], query: &str) -> Vec<usize> {
    let labels: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    match_indices(&labels, query)
}

/// Next/prev commit-file match index. `dir` 0 = first hit.
pub fn focus_commit_file_search(
    files: &[CommitFile],
    query: &str,
    current: usize,
    dir: i32,
) -> Option<usize> {
    let hits = collect_commit_file_match_indices(files, query);
    if hits.is_empty() {
        return None;
    }
    if dir == 0 {
        return hits.first().copied();
    }
    step_match_index(&hits, current, dir)
}

/// Indices of painted diff lines that contain `query`.
pub fn match_diff_line_indices(lines: &[String], query: &str) -> Vec<usize> {
    let labels: Vec<&str> = lines.iter().map(String::as_str).collect();
    match_indices(&labels, query)
}

/// Next/prev matching diff line. `dir` 0 = first hit.
pub fn focus_diff_search(
    lines: &[String],
    query: &str,
    current: Option<usize>,
    dir: i32,
) -> Option<usize> {
    let hits = match_diff_line_indices(lines, query);
    if hits.is_empty() {
        return None;
    }
    if dir == 0 {
        return hits.first().copied();
    }
    let cur = current.unwrap_or(usize::MAX);
    step_match_index(&hits, cur, dir)
}

/// Next/prev match index with wrap. If `current` is not a hit, jump to first
/// (`dir` > 0) or last (`dir` < 0).
pub fn step_match_index(indices: &[usize], current: usize, dir: i32) -> Option<usize> {
    if indices.is_empty() {
        return None;
    }
    let pos = indices.iter().position(|i| *i == current);
    let Some(pos) = pos else {
        return if dir < 0 {
            indices.last().copied()
        } else {
            indices.first().copied()
        };
    };
    let len = indices.len() as i32;
    let next = (pos as i32 + dir).rem_euclid(len) as usize;
    indices.get(next).copied()
}

/// Clamp a horizontal pan offset to `[0, max_offset]`.
pub fn clamp_col_offset(offset: i32, max_offset: usize) -> u16 {
    let max = max_offset as i32;
    offset.clamp(0, max.max(0)) as u16
}

/// Longest line minus the viewport. Never below 0.
pub fn max_col_offset(line_lens: &[usize], viewport_cols: usize) -> usize {
    let longest = line_lens.iter().copied().max().unwrap_or(0);
    longest.saturating_sub(viewport_cols.max(1))
}

/// Apply a pan delta and clamp.
pub fn apply_pan(offset: u16, delta: i32, max_offset: usize) -> u16 {
    clamp_col_offset(offset as i32 + delta, max_offset)
}

/// Slice `text` from `offset` for `width` columns (Unicode scalars).
pub fn slice_visible(text: &str, offset: usize, width: usize) -> String {
    text.chars().skip(offset).take(width).collect()
}

/// Skip `offset` display columns, then take up to `width` columns.
pub fn slice_cols(text: &str, offset: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut skipped = 0usize;
    let mut taken = 0usize;
    let mut out = String::new();
    for ch in text.chars() {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let cw = visible_width(s);
        if skipped < offset {
            skipped = skipped.saturating_add(cw);
            continue;
        }
        if taken.saturating_add(cw) > width {
            break;
        }
        out.push(ch);
        taken = taken.saturating_add(cw);
    }
    out
}

/// Cursor bar + indent + chevron+space. Shared with tree / commit-file paint.
pub fn list_prefix_cols(depth: usize) -> usize {
    1 + depth.saturating_mul(2) + 2
}

/// How far one segmented list row can pan.
pub fn list_row_pan_max(
    label_cols: usize,
    depth: usize,
    trailing_cols: usize,
    pane_cols: usize,
) -> usize {
    let pad = usize::from(trailing_cols > 0);
    let budget = pane_cols
        .saturating_sub(list_prefix_cols(depth))
        .saturating_sub(trailing_cols)
        .saturating_sub(pad)
        .max(1);
    label_cols.saturating_sub(budget)
}

/// First hunk header at or before `scroll`, else the line at `scroll`.
#[allow(dead_code)]
pub fn hunk_anchor(lines: &[String], scroll: usize) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let start = scroll.min(lines.len() - 1);
    for line in lines[..=start].iter().rev() {
        if line.starts_with("@@") {
            return Some(line.clone());
        }
    }
    for line in &lines[start..] {
        if line.starts_with("@@") {
            return Some(line.clone());
        }
    }
    Some(lines[start].clone())
}

/// Scroll so `anchor` stays in the upper third of `view_h`.
#[allow(dead_code)]
pub fn scroll_to_keep_anchor(lines: &[String], anchor: &str, view_h: u16) -> u16 {
    let Some(idx) = lines.iter().position(|l| l == anchor) else {
        return 0;
    };
    let h = view_h.max(1) as usize;
    let prefer = h / 3;
    idx.saturating_sub(prefer) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::tui::tree::{build_tree, default_folds, visible_for_tree, NodeKind};

    fn repo(name: &str, dirty_path: Option<&str>) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: dirty_path.is_some(),
            has_staged: false,
            has_untracked: false,
            changes: dirty_path
                .map(|path| {
                    vec![FileChange {
                        path: path.into(),
                        staged_status: None,
                        unstaged_status: Some("M".into()),
                        untracked: false,
                        old_path: None,
                    }]
                })
                .unwrap_or_default(),
            checkout_kind: crate::snapshot::CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
        }
    }

    fn tree(show_ignored: bool) -> crate::tui::tree::TreeNode {
        let built = build_workspace_snapshot(
            &[
                repo("app", Some("README.md")),
                repo("lib", None),
                repo("notes", Some("secret.md")),
            ],
            &["notes".into()],
            show_ignored,
            &[],
        );
        build_tree(&visible_for_tree(&built), true, "workspace")
    }

    #[test]
    fn next_and_prev_wrap() {
        let ids = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(step_match_id(&ids, Some("a"), 1).as_deref(), Some("b"));
        assert_eq!(step_match_id(&ids, Some("c"), 1).as_deref(), Some("a"));
        assert_eq!(step_match_id(&ids, Some("a"), -1).as_deref(), Some("c"));
        assert_eq!(step_match_id(&ids, None, 1).as_deref(), Some("a"));
        assert_eq!(step_match_id(&ids, None, -1).as_deref(), Some("c"));
    }

    #[test]
    fn first_match_unfolds_parent() {
        let tree = tree(false);
        let folds = default_folds(&tree);
        let (next_folds, id) = focus_tree_search(&tree, &folds, "README", None, 0);
        assert_eq!(id.as_deref(), Some("file:app:README.md"));
        let rows = flatten(&tree, &next_folds);
        assert!(rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn n_then_n_prev_walks_matches_and_unfolds() {
        let tree = tree(false);
        let mut folds = HashSet::new();
        folds.insert("repo:app".into());
        folds.insert("group:no-updates".into());
        let (folds, first) = focus_tree_search(&tree, &folds, "main", None, 0);
        assert!(first.is_some());
        let (folds, second) = focus_tree_search(&tree, &folds, "main", first.as_deref(), 1);
        assert_ne!(second, first);
        let (folds, prev) = focus_tree_search(&tree, &folds, "main", second.as_deref(), -1);
        assert_eq!(prev, first);
        if let Some(id) = prev {
            let rows = flatten(&tree, &folds);
            assert!(rows.iter().any(|r| r.id == id));
        }
    }

    #[test]
    fn hidden_ignored_is_not_a_search_hit() {
        let hidden = tree(false);
        assert!(collect_match_ids(&hidden, "secret").is_empty());
        assert!(collect_match_ids(&hidden, "notes").is_empty());
        let shown = tree(true);
        assert_eq!(
            collect_match_ids(&shown, "secret"),
            vec!["file:notes:secret.md".to_string()]
        );
        assert!(
            shown
                .children
                .iter()
                .any(|c| c.kind == NodeKind::Repo && c.chrome.path.contains("notes"))
                || flatten(&shown, &HashSet::new())
                    .iter()
                    .any(|r| r.label.contains("notes"))
        );
    }

    #[test]
    fn graph_subject_and_ref_are_searchable() {
        use workspace_status_graph::{
            format_relative_date, format_utc_timestamp, Commit, GraphRef,
        };
        let now = search_now_unix();
        let recent_unix = now - 90;
        let recent_utc = format_utc_timestamp(recent_unix);
        let recent_day = &recent_utc[..10];
        let rows = vec![
            GraphRow::Commit {
                commit: Commit {
                    id: "aa11bb22cc33dd44ee55ff6677889900aabbccdd".into(),
                    subject: "fix login timeout".into(),
                    parents: Vec::new(),
                    refs: vec![GraphRef::local("main"), GraphRef::tag("v9.9.9")],
                    author_name: "Ada SearchAuthor".into(),
                    author_date_unix: recent_unix,
                },
                is_head: true,
                worktrees: Vec::new(),
            },
            GraphRow::Commit {
                commit: Commit {
                    id: "bb22cc33dd44ee55ff6677889900aabbccddeeff".into(),
                    subject: "docs".into(),
                    parents: Vec::new(),
                    refs: vec![GraphRef::local("topic")],
                    author_name: String::new(),
                    author_date_unix: 1_700_000_000,
                },
                is_head: false,
                worktrees: Vec::new(),
            },
        ];
        assert_eq!(collect_graph_match_indices(&rows, "login"), vec![0]);
        assert_eq!(collect_graph_match_indices(&rows, "topic"), vec![1]);
        assert_eq!(collect_graph_match_indices(&rows, "v9.9.9"), vec![0]);
        assert_eq!(collect_graph_match_indices(&rows, "SearchAuthor"), vec![0]);
        assert_eq!(collect_graph_match_indices(&rows, "1m"), vec![0]);
        assert_eq!(collect_graph_match_indices(&rows, recent_day), vec![0]);
        assert_eq!(collect_graph_match_indices(&rows, "2023-11-14"), vec![1]);
        let old_painted = format_relative_date(1_700_000_000, now);
        assert_eq!(collect_graph_match_indices(&rows, &old_painted), vec![1]);
        assert_eq!(
            collect_graph_match_indices(&rows, "aa11bb22cc33dd44ee55ff6677889900aabbccdd"),
            vec![0]
        );
        assert_eq!(collect_graph_match_indices(&rows, "aa11bb2"), vec![0]);
        assert_eq!(focus_graph_search(&rows, "o", 0, 1), Some(1));
        assert_eq!(focus_graph_search(&rows, "o", 1, -1), Some(0));
    }

    #[test]
    fn graph_stash_and_worktree_search_include_sha_author_time() {
        use workspace_status_graph::{Stash, Worktree};
        let stash_rows = vec![GraphRow::Stash(Stash {
            id: "deadbeefcafebabe".into(),
            stash_ref: "stash@{0}".into(),
            subject: "wip notes".into(),
            author_name: "UniqueAuthorXYZ".into(),
            author_date_unix: 1_700_000_000,
            parent_id: None,
        })];
        assert_eq!(collect_graph_match_indices(&stash_rows, "wip"), vec![0]);
        assert_eq!(
            collect_graph_match_indices(&stash_rows, "stash@{0}"),
            vec![0]
        );
        assert_eq!(
            collect_graph_match_indices(&stash_rows, "UniqueAuthorXYZ"),
            vec![0]
        );
        assert_eq!(collect_graph_match_indices(&stash_rows, "deadbee"), vec![0]);
        assert_eq!(
            collect_graph_match_indices(&stash_rows, "2023-11-14"),
            vec![0]
        );

        let wt_rows = vec![GraphRow::Worktree(Worktree {
            path: "feature-wt".into(),
            head_id: Some("ffeeddccbbaa99887766554433221100ffeeddcc".into()),
            branch: Some("feature/login-page".into()),
            ignored: false,
            is_current: false,
        })];
        assert_eq!(collect_graph_match_indices(&wt_rows, "feature-wt"), vec![0]);
        assert_eq!(collect_graph_match_indices(&wt_rows, "login-page"), vec![0]);
        assert_eq!(collect_graph_match_indices(&wt_rows, "ffeeddc"), vec![0]);
    }

    #[test]
    fn commit_file_and_diff_search_and_pan_clamp() {
        let files = vec![
            CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
            },
            CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
            },
        ];
        assert_eq!(focus_commit_file_search(&files, "lib", 0, 0), Some(1));
        let lines = vec![
            "@@ hunk @@".into(),
            "+needle line".into(),
            " context".into(),
        ];
        assert_eq!(focus_diff_search(&lines, "needle", None, 0), Some(1));
        assert_eq!(apply_pan(0, -1, 4), 0);
        assert_eq!(apply_pan(0, 1, 4), 1);
        assert_eq!(apply_pan(4, 1, 4), 4);
        assert_eq!(slice_cols("abcdefghij", 3, 4), "defg");
        assert_eq!(slice_cols("abcdefghij", 0, 4), "abcd");
        assert_eq!(list_row_pan_max(20, 0, 0, 10), 13);
        assert_eq!(hunk_anchor(&lines, 1).as_deref(), Some("@@ hunk @@"));
        assert_eq!(scroll_to_keep_anchor(&lines, "@@ hunk @@", 9), 0);
    }
}

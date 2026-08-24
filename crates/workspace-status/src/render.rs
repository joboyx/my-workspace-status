//! `--plain` renderer.

use std::collections::{BTreeMap, HashMap};

use crate::helpers::{
    extract_ticket_id, format_branch_with_merge, format_checkout_repo_label, is_attention_sync_note,
    sorted_unique, visible_width,
};
use crate::snapshot::{FileChange, RepoSnapshot, SummaryState, VerboseRow};

fn pad_visible(value: &str, width: usize) -> String {
    let pad = width.saturating_sub(visible_width(value));
    format!("{value}{}", " ".repeat(pad))
}

fn badge_for_change(change: &FileChange) -> &'static str {
    if change.unstaged_status.as_deref() == Some("U") || change.staged_status.as_deref() == Some("U")
    {
        return "⚠️U";
    }
    if change.staged_status.is_some() && change.unstaged_status.is_some() {
        return "🟠MS";
    }
    let status = change
        .unstaged_status
        .as_deref()
        .or(change.staged_status.as_deref());
    match status {
        Some("R") => "🟣R",
        Some("D") => "🔴D",
        Some("A") => "🟢A",
        Some(_) if change.staged_status.is_some() => "🔵S",
        _ if change.untracked => "🟢A",
        _ => "🟡M",
    }
}

fn file_display(change: &FileChange) -> String {
    let name = change.path.rsplit('/').next().unwrap_or(&change.path);
    if let Some(old) = &change.old_path {
        let old_name = old.rsplit('/').next().unwrap_or(old);
        format!("{} {old_name} -> {name}", badge_for_change(change))
    } else {
        format!("{} {name}", badge_for_change(change))
    }
}

#[derive(Default)]
struct FileTreeNode {
    dirs: BTreeMap<String, FileTreeNode>,
    files: Vec<FileChange>,
}

fn add_tree_change(root: &mut FileTreeNode, change: &FileChange) {
    let mut parts: Vec<&str> = change.path.split('/').filter(|s| !s.is_empty()).collect();
    let Some(file_name) = parts.pop() else {
        return;
    };
    let _ = file_name;
    let mut node = root;
    for dir in parts {
        node = node.dirs.entry(dir.to_string()).or_default();
    }
    node.files.push(change.clone());
}

fn collapse_node<'a>(name: String, node: &'a FileTreeNode) -> (String, &'a FileTreeNode) {
    let mut collapsed_name = name;
    let mut collapsed_node = node;
    while collapsed_node.files.is_empty() && collapsed_node.dirs.len() == 1 {
        let (child_name, child_node) = collapsed_node.dirs.iter().next().unwrap();
        collapsed_name = format!("{collapsed_name}/{child_name}");
        collapsed_node = child_node;
    }
    (collapsed_name, collapsed_node)
}

fn render_tree_node(node: &FileTreeNode, prefix: &str) -> Vec<String> {
    let mut dir_entries: Vec<(String, &FileTreeNode)> = node
        .dirs
        .iter()
        .map(|(name, child)| collapse_node(name.clone(), child))
        .collect();
    dir_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut file_entries = node.files.clone();
    file_entries.sort_by(|a, b| a.path.cmp(&b.path));

    let mut items: Vec<(String, Option<&FileTreeNode>)> = dir_entries
        .into_iter()
        .map(|(n, c)| (n, Some(c)))
        .collect();
    items.extend(file_entries.iter().map(|c| (file_display(c), None)));

    let mut lines = Vec::new();
    let last_i = items.len().saturating_sub(1);
    for (index, (label, child)) in items.iter().enumerate() {
        let last = index == last_i;
        lines.push(format!("{prefix}{} {label}", if last { "└─" } else { "├─" }));
        if let Some(child) = child {
            let next = format!("{prefix}{}", if last { "   " } else { "│  " });
            lines.extend(render_tree_node(child, &next));
        }
    }
    lines
}

fn repo_label(snapshot: &RepoSnapshot) -> String {
    let repo = format_checkout_repo_label(snapshot);
    if let Some(ticket) = extract_ticket_id(&snapshot.branch) {
        format!("{repo} ({ticket})")
    } else {
        repo
    }
}

fn sync_label(snapshot: &RepoSnapshot) -> String {
    let repo = format_checkout_repo_label(snapshot);
    let ticket = extract_ticket_id(&snapshot.branch);
    let mut label = format!("{repo} [{}]", snapshot.branch);
    if let Some(ticket) = ticket {
        label.push_str(&format!(" ({ticket})"));
    }
    if !snapshot.sync_note.is_empty() {
        label.push_str(&format!(" - {}", snapshot.sync_note));
    }
    label
}

fn branch_summary_label(snapshot: &RepoSnapshot) -> String {
    let repo = format_checkout_repo_label(snapshot);
    let ticket = extract_ticket_id(&snapshot.branch);
    let base = if let Some(ticket) = ticket {
        format!("{repo} ({ticket})")
    } else {
        format!("{repo} [{}]", snapshot.branch)
    };
    format_branch_with_merge(&base, snapshot.merged_into_default)
}

fn render_repo_change_lines(snapshot: &RepoSnapshot) -> Vec<String> {
    let mut root = FileTreeNode::default();
    for change in &snapshot.changes {
        add_tree_change(&mut root, change);
    }
    let mut lines = vec![format!("  📦 {}", repo_label(snapshot))];
    lines.extend(render_tree_node(&root, "     "));
    lines
}

fn render_verbose(rows: &[VerboseRow], repo_width: usize, branch_width: usize) -> Vec<String> {
    let sync_width = rows
        .iter()
        .map(|r| visible_width(&r.sync))
        .max()
        .unwrap_or(0)
        .max(visible_width("Sync"));
    let files_width = rows
        .iter()
        .map(|r| visible_width(&r.files))
        .max()
        .unwrap_or(0)
        .max(visible_width("Files"));
    let mut lines = vec![format!(
        "{}  {}  {}  {}",
        pad_visible("Repo", repo_width),
        pad_visible("Branch", branch_width),
        pad_visible("Sync", sync_width),
        pad_visible("Files", files_width)
    )];
    for r in rows {
        let mut line = format!(
            "{}  {}  {}  {}",
            pad_visible(&r.repo, repo_width),
            pad_visible(&r.branch, branch_width),
            pad_visible(&r.sync, sync_width),
            pad_visible(&r.files, files_width)
        );
        if !r.note.is_empty() {
            line.push_str(&format!("  {}", r.note));
        }
        lines.push(line);
    }
    lines
}

fn append_section_gap(lines: &mut Vec<String>) {
    if !lines.is_empty() {
        lines.push(String::new());
    }
}

fn append_sync_group(
    lines: &mut Vec<String>,
    label: &str,
    repos: &[String],
    map: &HashMap<String, RepoSnapshot>,
) {
    if repos.is_empty() {
        return;
    }
    lines.push(label.to_string());
    for repo in repos {
        if let Some(snapshot) = map.get(repo) {
            lines.push(format!("    - {}", sync_label(snapshot)));
        }
    }
}

fn append_branch_group(
    lines: &mut Vec<String>,
    label: &str,
    repos: &[String],
    map: &HashMap<String, RepoSnapshot>,
) {
    if repos.is_empty() {
        return;
    }
    lines.push(label.to_string());
    for repo in repos {
        if let Some(snapshot) = map.get(repo) {
            lines.push(format!("    - {}", branch_summary_label(snapshot)));
        }
    }
}

fn append_linked_section(
    lines: &mut Vec<String>,
    linked: &[String],
    map: &HashMap<String, RepoSnapshot>,
) {
    if linked.is_empty() {
        return;
    }
    append_section_gap(lines);
    lines.push(format!("🔗 Linked worktrees ({}):", linked.len()));
    for repo in linked {
        if let Some(snapshot) = map.get(repo) {
            lines.push(format!("    - {}", branch_summary_label(snapshot)));
        }
    }
}

pub fn render_workspace_status(
    snapshots: &[RepoSnapshot],
    summary: &SummaryState,
    verbose: &(Vec<VerboseRow>, Vec<VerboseRow>, Vec<VerboseRow>, usize, usize),
    show_verbose: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    let map: HashMap<String, RepoSnapshot> = snapshots
        .iter()
        .map(|s| (s.repo.clone(), s.clone()))
        .collect();
    let linked = sorted_unique(summary.linked_worktrees.iter().cloned());

    if show_verbose {
        let mut rows = Vec::new();
        rows.extend(verbose.0.clone());
        rows.extend(verbose.1.clone());
        rows.extend(verbose.2.clone());
        lines.extend(render_verbose(&rows, verbose.3, verbose.4));
    }

    if snapshots.is_empty() {
        append_section_gap(&mut lines);
        lines.push("ℹ️ No git repos found".to_string());
        return lines;
    }

    let total_changes = summary.changes_uncommitted.len()
        + summary.changes_staged.len()
        + summary.changes_both.len()
        + summary.changes_untracked.len();
    let total_sync =
        summary.sync_behind.len() + summary.sync_ahead.len() + summary.sync_diverged.len();
    let total_branches = summary.branch_feature.len()
        + summary.branch_bugfix.len()
        + summary.branch_chore.len()
        + summary.branch_release.len()
        + summary.branch_unknown.len();
    let mut attention: Vec<&RepoSnapshot> = snapshots
        .iter()
        .filter(|s| is_attention_sync_note(&s.sync_note))
        .collect();
    attention.sort_by(|a, b| a.repo.cmp(&b.repo));

    if total_changes == 0 && total_sync == 0 && total_branches == 0 && attention.is_empty() {
        append_section_gap(&mut lines);
        lines.push("✅ All repos clean and up-to-date".to_string());
        append_linked_section(&mut lines, &linked, &map);
        return lines;
    }

    if total_changes > 0 {
        let repos = sorted_unique(
            summary
                .changes_uncommitted
                .iter()
                .chain(&summary.changes_staged)
                .chain(&summary.changes_both)
                .chain(&summary.changes_untracked)
                .cloned(),
        );
        append_section_gap(&mut lines);
        lines.push("File changes".to_string());
        for (index, repo) in repos.iter().enumerate() {
            if let Some(snapshot) = map.get(repo) {
                if index > 0 {
                    lines.push(String::new());
                }
                lines.extend(render_repo_change_lines(snapshot));
            }
        }
    }

    if total_sync > 0 {
        append_section_gap(&mut lines);
        lines.push(format!("🔄 Sync status ({total_sync}):"));
        append_sync_group(
            &mut lines,
            "  ⬇️ behind:",
            &sorted_unique(summary.sync_behind.iter().cloned()),
            &map,
        );
        append_sync_group(
            &mut lines,
            "  ⬆️ ahead:",
            &sorted_unique(summary.sync_ahead.iter().cloned()),
            &map,
        );
        append_sync_group(
            &mut lines,
            "  🔀 diverged:",
            &sorted_unique(summary.sync_diverged.iter().cloned()),
            &map,
        );
    }

    if total_branches > 0 {
        append_section_gap(&mut lines);
        lines.push(format!("🌿 Branches ({total_branches}):"));
        append_branch_group(
            &mut lines,
            "  🚧 feature:",
            &sorted_unique(summary.branch_feature.iter().cloned()),
            &map,
        );
        append_branch_group(
            &mut lines,
            "  🐛 bugfix:",
            &sorted_unique(summary.branch_bugfix.iter().cloned()),
            &map,
        );
        append_branch_group(
            &mut lines,
            "  🔧 chore:",
            &sorted_unique(summary.branch_chore.iter().cloned()),
            &map,
        );
        append_branch_group(
            &mut lines,
            "  🚀 release:",
            &sorted_unique(summary.branch_release.iter().cloned()),
            &map,
        );
        append_branch_group(
            &mut lines,
            "  🌿 unknown:",
            &sorted_unique(summary.branch_unknown.iter().cloned()),
            &map,
        );
    }

    if !attention.is_empty() {
        append_section_gap(&mut lines);
        lines.push(format!("⚠️ Attention ({}):", attention.len()));
        for snapshot in attention {
            lines.push(format!("    - {}", sync_label(snapshot)));
        }
    }

    append_linked_section(&mut lines, &linked, &map);
    lines
}

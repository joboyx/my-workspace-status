//! Branch/sync classification and string helpers.

use crate::snapshot::{CheckoutKind, RepoSnapshot, SyncStatus};

pub const DETACHED_HEAD_BRANCH: &str = "HEAD (detached)";
/// Porcelain-parse failure. Not a `refs/heads` name.
pub const UNKNOWN_HEAD_BRANCH: &str = "(unknown)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Default,
    Feature,
    Bugfix,
    Chore,
    Release,
    Unknown,
}

/// Normalize a user-supplied repo filter to a workspace-relative path.
pub fn normalize_filter_repo(arg: &str) -> String {
    let mut s = arg.trim().replace('\\', "/");
    while s.starts_with("./") {
        s = s[2..].to_string();
    }
    while s.starts_with('/') {
        s = s[1..].to_string();
    }
    while s.ends_with('/') {
        s.pop();
    }
    s
}

pub fn is_default_branch(branch: &str, override_name: Option<&str>) -> bool {
    if let Some(name) = override_name {
        return branch == name;
    }
    matches!(branch, "main" | "master" | "develop")
}

pub fn get_branch_kind(branch: &str, override_name: Option<&str>) -> BranchKind {
    if is_default_branch(branch, override_name) {
        return BranchKind::Default;
    }
    if branch.starts_with("feature/") {
        return BranchKind::Feature;
    }
    if branch.starts_with("bugfix/") {
        return BranchKind::Bugfix;
    }
    if branch.starts_with("chore/") {
        return BranchKind::Chore;
    }
    if branch.starts_with("release/") {
        return BranchKind::Release;
    }
    BranchKind::Unknown
}

pub fn get_branch_priority(branch: &str) -> i32 {
    match branch {
        "main" => 1,
        "master" => 2,
        "develop" => 3,
        _ => 4,
    }
}

pub fn get_sync_priority(status: SyncStatus) -> i32 {
    match status {
        SyncStatus::UpToDate => 0,
        SyncStatus::NoUpstream => 1,
        SyncStatus::Behind => 2,
        SyncStatus::Ahead => 3,
        SyncStatus::Diverged => 4,
    }
}

pub fn get_branch_emoji(branch: &str) -> &'static str {
    if branch == "main" || branch == "master" {
        return "🔥";
    }
    match get_branch_kind(branch, None) {
        BranchKind::Feature => "🚧",
        BranchKind::Bugfix => "🐛",
        BranchKind::Chore => "🔧",
        BranchKind::Release => "🚀",
        _ => "🌿",
    }
}

pub fn is_attention_sync_note(note: &str) -> bool {
    note == "no commits yet" || note == "status failed"
}

pub fn is_detached_head_branch(branch: &str) -> bool {
    branch.is_empty()
        || branch == DETACHED_HEAD_BRANCH
        || branch == "HEAD"
        || branch == "(detached)"
}

/// True when `name` is a real local branch, not detached / `(unknown)`.
pub fn is_counted_local_branch(name: &str) -> bool {
    !is_detached_head_branch(name) && name != UNKNOWN_HEAD_BRANCH
}

pub fn extract_ticket_id(branch: &str) -> Option<&str> {
    let bytes = branch.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_uppercase() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'-' {
                let digits_start = i + 1;
                i += 1;
                let mut digits = 0;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    digits += 1;
                    i += 1;
                }
                if digits > 0 {
                    return Some(&branch[start..i]);
                }
                i = digits_start;
                continue;
            }
            continue;
        }
        i += 1;
    }
    None
}

pub fn format_merge_mark(merged: Option<bool>) -> &'static str {
    match merged {
        Some(true) => " ✅",
        Some(false) => " 🌱",
        None => "",
    }
}

pub fn format_checkout_repo_label(snapshot: &RepoSnapshot) -> String {
    if snapshot.checkout_kind == CheckoutKind::Linked {
        format!("🔗 {}", snapshot.repo)
    } else {
        snapshot.repo.clone()
    }
}

pub fn format_branch_with_merge(label: &str, merged: Option<bool>) -> String {
    format!("{label}{}", format_merge_mark(merged))
}

pub fn sorted_unique(repos: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = repos.into_iter().filter(|s| !s.is_empty()).collect();
    out.sort();
    out.dedup();
    out
}

pub fn compare_repo_paths_for_display(a: &RepoSnapshot, b: &RepoSnapshot) -> std::cmp::Ordering {
    let a_primary = a.primary_repo.as_deref().unwrap_or(&a.repo);
    let b_primary = b.primary_repo.as_deref().unwrap_or(&b.repo);
    a_primary
        .cmp(b_primary)
        .then_with(|| {
            let a_linked = i32::from(a.checkout_kind == CheckoutKind::Linked);
            let b_linked = i32::from(b.checkout_kind == CheckoutKind::Linked);
            a_linked.cmp(&b_linked)
        })
        .then_with(|| a.repo.cmp(&b.repo))
}

/// Terminal display width.
pub fn visible_width(value: &str) -> usize {
    let mut width = 0;
    for ch in value.chars() {
        let code = ch as u32;
        if code == 0xfe0f || code == 0x200d {
            continue;
        }
        let wide = (0x1100..=0x115f).contains(&code)
            || code == 0x2329
            || code == 0x232a
            || (0x2190..=0x21ff).contains(&code)
            || (0x2300..=0x23ff).contains(&code)
            || (0x2600..=0x27bf).contains(&code)
            || (0x2b00..=0x2bff).contains(&code)
            || (0x2e80..=0xa4cf).contains(&code)
            || (0xac00..=0xd7a3).contains(&code)
            || (0xf900..=0xfaff).contains(&code)
            || (0xfe10..=0xfe19).contains(&code)
            || (0xfe30..=0xfe6f).contains(&code)
            || (0xff00..=0xff60).contains(&code)
            || (0xffe0..=0xffe6).contains(&code)
            || (0x1f000..=0x1faff).contains(&code);
        width += if wide { 2 } else { 1 };
    }
    width
}

pub fn sanitize_path(p: &str) -> String {
    p.chars()
        .filter(|c| {
            let code = *c as u32;
            code >= 0x20 && code != 0x7f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_dot_slash_and_trailing_slash() {
        assert_eq!(normalize_filter_repo("./app/"), "app");
        assert_eq!(normalize_filter_repo("notes"), "notes");
    }

    #[test]
    fn ticket_id_from_branch() {
        assert_eq!(
            extract_ticket_id("feature/ABCD-1234-rust-cli"),
            Some("ABCD-1234")
        );
        assert_eq!(extract_ticket_id("main"), None);
    }

    #[test]
    fn default_branch_override_wins() {
        assert!(!is_default_branch("main", Some("develop")));
        assert!(is_default_branch("develop", Some("develop")));
    }
}

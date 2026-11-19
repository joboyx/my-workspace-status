//! Tree focus: narrow the left-pane tree to one feature or repo.
//!
//! A focus is one query of comma-separated terms. A checkout is in focus
//! when a term is a case-insensitive substring of its family's primary repo
//! path, its own checkout path, or its branch name. Repo families (a
//! primary checkout plus its linked worktrees, keyed like
//! [`super::tree::build_tree`] by `primary_repo` else `repo`) are the unit
//! of the split: a family with at least one kept checkout is focused, every
//! other family is outside the focus.
//!
//! This module is pure: no I/O, no app state. The tree, the key and palette
//! rows, and the workspace-row actions read it; the query itself persists
//! per workspace root through
//! [`super::ui_state_store::load_tree_focus`] and
//! [`super::ui_state_store::save_tree_focus`].

use std::collections::BTreeMap;
use std::ops::Range;

use crate::helpers::{is_counted_local_branch, is_default_branch};
use crate::snapshot::{WorkspaceRepoSnapshot, WorkspaceSnapshot};

/// A parsed focus query.
///
/// The raw query is what the user typed (and what the UI-state file
/// stores); the terms are the query split on `,`, trimmed, lower-cased, with
/// empty terms dropped. A query with no terms is "no focus"
/// ([`TreeFocus::is_empty`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeFocus {
    query: String,
    terms: Vec<String>,
}

impl TreeFocus {
    /// Parse `query`: split on `,`, trim each term, drop empty terms, and
    /// lower-case the rest. `"ABC-123, billing"` has the terms `abc-123`
    /// and `billing`; `" , "` has none.
    pub fn parse(query: &str) -> Self {
        let terms = query
            .split(',')
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .map(str::to_lowercase)
            .collect();
        Self {
            query: query.to_string(),
            terms,
        }
    }

    /// The query as typed, before the split.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The lower-case terms the query matches with (OR).
    pub fn terms(&self) -> &[String] {
        &self.terms
    }

    /// True when the query has no terms: no focus is set.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// True when any term is a case-insensitive substring of `text`.
    pub fn matches(&self, text: &str) -> bool {
        if self.terms.is_empty() {
            return false;
        }
        let lower = text.to_lowercase();
        self.terms.iter().any(|term| lower.contains(term.as_str()))
    }
}

/// True when the focus keeps one checkout of a family.
///
/// A term that matches `family_primary_path` keeps every checkout of the
/// family (a repo focus keeps the whole family). Otherwise the checkout is
/// kept when a term matches its `branch` or its own `checkout_path`. An
/// empty focus keeps every checkout.
pub fn checkout_kept(
    focus: &TreeFocus,
    family_primary_path: &str,
    checkout_path: &str,
    branch: &str,
) -> bool {
    focus.is_empty()
        || focus.matches(family_primary_path)
        || focus.matches(branch)
        || focus.matches(checkout_path)
}

/// True when the focus keeps at least one of `members`, the checkouts of
/// the family whose primary repo path is `family_primary_path`. A family
/// that is not kept is outside the focus.
pub fn family_kept(
    focus: &TreeFocus,
    family_primary_path: &str,
    members: &[&WorkspaceRepoSnapshot],
) -> bool {
    members
        .iter()
        .any(|m| checkout_kept(focus, family_primary_path, &m.repo, &m.branch))
}

/// One family in the focused part of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusedFamily {
    /// The family key: the primary repo path (a flat repo's own path).
    pub primary_path: String,
    /// Paths of the checkouts the focus keeps, in snapshot order. Never
    /// empty. The tree leaves the other checkouts of the family out.
    pub kept_checkout_paths: Vec<String>,
    /// All checkouts of the family in the snapshot, kept or not: the `n`
    /// of the family's `k/n wt` mark.
    pub checkout_count: usize,
}

impl FocusedFamily {
    /// True when the focus keeps only some checkouts, so the family
    /// container shows `k/n wt`.
    pub fn is_partial(&self) -> bool {
        self.kept_checkout_paths.len() < self.checkout_count
    }

    /// True when the focus keeps the checkout at `path`.
    pub fn keeps(&self, path: &str) -> bool {
        self.kept_checkout_paths.iter().any(|kept| kept == path)
    }
}

/// Family counts for the focus chip and the outside-focus row.
///
/// The unit is a top-level repo: a family (a flat repo is a family of one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FocusCounts {
    /// Families with at least one kept checkout (chip `focused/total`).
    pub focused: usize,
    /// All families of the snapshot (chip `focused/total`).
    pub total: usize,
    /// Families with no kept checkout (`N outside focus`).
    pub outside: usize,
    /// Outside families where any checkout has a staged, unstaged, or
    /// untracked change (`M dirty`).
    pub outside_dirty: usize,
}

/// A snapshot split by a focus into focused and outside families.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusSplit {
    /// Focused families, ordered by primary repo path.
    pub focused: Vec<FocusedFamily>,
    /// Primary repo paths of the outside families, in the same order.
    pub outside: Vec<String>,
    /// The chip and outside-row counts.
    pub counts: FocusCounts,
}

/// Split `visible` by `focus` into focused families (with their kept
/// checkouts) and outside families, and count them.
///
/// `visible` is the snapshot after the positional repo filter and the
/// `hiddenPaths` filter, so hidden repos that are not shown count in
/// neither part. Families are keyed like the tree: `primary_repo`, else the
/// checkout's own `repo` path. An empty focus puts every family and every
/// checkout in the focused part.
pub fn split_by_focus(focus: &TreeFocus, visible: &WorkspaceSnapshot) -> FocusSplit {
    let mut families: BTreeMap<&str, Vec<&WorkspaceRepoSnapshot>> = BTreeMap::new();
    for repo in &visible.repos {
        let key = repo.primary_repo.as_deref().unwrap_or(&repo.repo);
        families.entry(key).or_default().push(repo);
    }
    let mut split = FocusSplit::default();
    for (primary_path, members) in families {
        let kept_checkout_paths: Vec<String> = members
            .iter()
            .filter(|m| checkout_kept(focus, primary_path, &m.repo, &m.branch))
            .map(|m| m.repo.clone())
            .collect();
        if kept_checkout_paths.is_empty() {
            if members.iter().any(|m| checkout_is_dirty(m)) {
                split.counts.outside_dirty += 1;
            }
            split.outside.push(primary_path.to_string());
        } else {
            split.focused.push(FocusedFamily {
                primary_path: primary_path.to_string(),
                kept_checkout_paths,
                checkout_count: members.len(),
            });
        }
    }
    split.counts.focused = split.focused.len();
    split.counts.outside = split.outside.len();
    split.counts.total = split.counts.focused + split.counts.outside;
    split
}

/// True when the checkout has a staged, unstaged, or untracked change.
fn checkout_is_dirty(repo: &WorkspaceRepoSnapshot) -> bool {
    repo.has_staged || repo.has_unstaged || repo.has_untracked
}

/// The "this feature" preset: the ticket-like token of `branch`.
///
/// The token is the first match of `[A-Za-z][A-Za-z0-9]*-[0-9]+`
/// (`feature/ABC-123-invoice-export` → `ABC-123`, `bugfix/web-42-y` →
/// `web-42`). With no such match it is the branch name with its first
/// `<prefix>/` removed (`feature/auth-refresh` → `auth-refresh`; a branch
/// with no `/` is its own token). `None` on the default branch (see
/// [`is_default_branch`] with `default_branch_override`), a detached HEAD,
/// and an unknown HEAD (see [`is_counted_local_branch`]).
pub fn feature_token(branch: &str, default_branch_override: Option<&str>) -> Option<String> {
    if !is_counted_local_branch(branch) || is_default_branch(branch, default_branch_override) {
        return None;
    }
    if let Some(ticket) = ticket_like_token(branch) {
        return Some(ticket.to_string());
    }
    let rest = branch.split_once('/').map_or(branch, |(_, rest)| rest);
    let token = if rest.is_empty() { branch } else { rest };
    Some(token.to_string())
}

/// First match of `[A-Za-z][A-Za-z0-9]*-[0-9]+` in `text`.
///
/// The letter-digit run cannot hold `-`, so the longest run from each start
/// is the only candidate there; the scan tries each start left to right,
/// like a leftmost regex match.
fn ticket_like_token(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_alphabetic() {
            continue;
        }
        let mut end = start + 1;
        while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
            end += 1;
        }
        if bytes.get(end) != Some(&b'-') {
            continue;
        }
        let digits_start = end + 1;
        let mut digits_end = digits_start;
        while digits_end < bytes.len() && bytes[digits_end].is_ascii_digit() {
            digits_end += 1;
        }
        if digits_end > digits_start {
            return Some(&text[start..digits_end]);
        }
    }
    None
}

/// The "this repo" preset: the family primary repo path of a row.
///
/// `checkout_path` is the row's checkout path (the tree row `repo`; `None`
/// on workspace and group rows, where the preset is unavailable).
/// `primary_repo` is the row's `primary_repo` (set on a linked checkout and
/// its file rows). `app/.worktrees/feat-login` with primary `app` gives
/// `app`, so a repo focus keeps the whole family.
pub fn repo_preset(checkout_path: Option<&str>, primary_repo: Option<&str>) -> Option<String> {
    primary_repo.or(checkout_path).map(str::to_string)
}

/// Byte ranges of `text` that a focus term matches, for the highlight.
///
/// Matching is case-insensitive (Unicode lower case); each range starts and
/// ends on a char boundary of `text`. Every occurrence of every term counts,
/// overlapping ones included; the result is sorted, and overlapping or
/// touching ranges are merged, so no two ranges overlap. Empty for an empty
/// focus or no match.
pub fn match_ranges(focus: &TreeFocus, text: &str) -> Vec<Range<usize>> {
    if focus.is_empty() || text.is_empty() {
        return Vec::new();
    }
    // Lower-case `text` and keep, for each lower-case byte, the byte range
    // of the source char it came from: a lower-case char can have another
    // byte length than its source char.
    let mut lower = String::with_capacity(text.len());
    let mut source: Vec<Range<usize>> = Vec::with_capacity(text.len());
    for (at, ch) in text.char_indices() {
        let char_range = at..at + ch.len_utf8();
        for lower_ch in ch.to_lowercase() {
            lower.push(lower_ch);
            for _ in 0..lower_ch.len_utf8() {
                source.push(char_range.clone());
            }
        }
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for term in focus.terms() {
        let mut from = 0;
        while let Some(found) = lower[from..].find(term.as_str()) {
            let start = from + found;
            let end = start + term.len();
            ranges.push(source[start].start..source[end - 1].end);
            let step = lower[start..].chars().next().map_or(1, char::len_utf8);
            from = start + step;
        }
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, visible_workspace_snapshot, CheckoutKind, RepoSnapshot,
    };
    use crate::testutil::{repo, repo_snapshot};

    /// A linked worktree of `primary` at `path` on `branch`.
    fn linked(path: &str, primary: &str, branch: &str) -> RepoSnapshot {
        RepoSnapshot {
            branch: branch.to_string(),
            checkout_kind: CheckoutKind::Linked,
            primary_repo: Some(primary.to_string()),
            ..repo_snapshot(path)
        }
    }

    /// A primary checkout at `path` on `branch`.
    fn on_branch(path: &str, branch: &str) -> RepoSnapshot {
        RepoSnapshot {
            branch: branch.to_string(),
            ..repo_snapshot(path)
        }
    }

    /// The demo-like workspace: `app` with two linked worktrees, `services/api`
    /// on a feature branch, clean `lib`, dirty `notes`.
    fn workspace() -> WorkspaceSnapshot {
        build_workspace_snapshot(
            &[
                repo_snapshot("app"),
                linked(
                    "app/.worktrees/abc-123",
                    "app",
                    "feature/ABC-123-invoice-export",
                ),
                linked("app/.worktrees/login", "app", "feature/login"),
                on_branch("services/api", "feature/abc-123-api"),
                repo("lib", false),
                repo("notes", true),
            ],
            &[],
            false,
            &[],
        )
    }

    fn family(primary: &str, kept: &[&str], count: usize) -> FocusedFamily {
        FocusedFamily {
            primary_path: primary.to_string(),
            kept_checkout_paths: kept.iter().map(|p| p.to_string()).collect(),
            checkout_count: count,
        }
    }

    #[test]
    fn parse_splits_on_commas_trims_lowercases_and_drops_empty_terms() {
        let focus = TreeFocus::parse(" ABC-123 , Billing,, ");
        assert_eq!(focus.query(), " ABC-123 , Billing,, ");
        assert_eq!(focus.terms(), ["abc-123", "billing"]);
        assert!(!focus.is_empty());
        for blank in ["", "   ", ",", " , ,"] {
            let focus = TreeFocus::parse(blank);
            assert!(focus.is_empty(), "{blank:?}");
            assert_eq!(focus.terms(), [] as [&str; 0], "{blank:?}");
        }
    }

    #[test]
    fn matcher_is_a_case_insensitive_substring_with_comma_or() {
        let focus = TreeFocus::parse("abc-123, LIB");
        assert!(focus.matches("feature/ABC-123-invoice-export"));
        assert!(focus.matches("services/lib-tools"));
        assert!(focus.matches("Lib"));
        assert!(!focus.matches("feature/ABC-12"));
        assert!(!focus.matches("app"));
        assert!(!TreeFocus::parse(" , ").matches("anything"));
    }

    #[test]
    fn checkout_kept_by_primary_path_branch_or_own_path() {
        let focus = TreeFocus::parse("abc-123");
        assert!(checkout_kept(
            &focus,
            "app",
            "app/.worktrees/x",
            "feature/ABC-123-x"
        ));
        assert!(checkout_kept(
            &focus,
            "app",
            "app/.worktrees/abc-123",
            "feature/other"
        ));
        assert!(!checkout_kept(&focus, "app", "app", "main"));
        let repo_focus = TreeFocus::parse("app");
        assert!(checkout_kept(
            &repo_focus,
            "app",
            "app/.worktrees/x",
            "feature/y"
        ));
        assert!(checkout_kept(&TreeFocus::default(), "lib", "lib", "main"));
    }

    #[test]
    fn family_kept_when_any_member_is_kept() {
        let snapshot = workspace();
        let app: Vec<&WorkspaceRepoSnapshot> = snapshot
            .repos
            .iter()
            .filter(|r| r.repo.starts_with("app"))
            .collect();
        assert_eq!(app.len(), 3);
        assert!(family_kept(&TreeFocus::parse("login"), "app", &app));
        assert!(!family_kept(&TreeFocus::parse("billing"), "app", &app));
    }

    #[test]
    fn feature_focus_keeps_only_matching_worktrees_across_repos() {
        let split = split_by_focus(&TreeFocus::parse("ABC-123"), &workspace());
        assert_eq!(
            split.focused,
            vec![
                family("app", &["app/.worktrees/abc-123"], 3),
                family("services/api", &["services/api"], 1),
            ]
        );
        assert!(split.focused[0].is_partial(), "app shows 1/3 wt");
        assert!(split.focused[0].keeps("app/.worktrees/abc-123"));
        assert!(!split.focused[0].keeps("app"));
        assert!(!split.focused[1].is_partial());
        assert_eq!(split.outside, ["lib", "notes"]);
        assert_eq!(
            split.counts,
            FocusCounts {
                focused: 2,
                total: 4,
                outside: 2,
                outside_dirty: 1,
            }
        );
    }

    #[test]
    fn repo_term_keeps_the_whole_family() {
        let split = split_by_focus(&TreeFocus::parse("app"), &workspace());
        assert_eq!(
            split.focused,
            vec![family(
                "app",
                &["app", "app/.worktrees/abc-123", "app/.worktrees/login"],
                3
            )]
        );
        assert!(!split.focused[0].is_partial());
        assert_eq!(split.outside, ["lib", "notes", "services/api"]);
    }

    #[test]
    fn a_linked_only_match_focuses_the_family_without_its_primary() {
        let split = split_by_focus(&TreeFocus::parse("feature/login"), &workspace());
        assert_eq!(
            split.focused,
            vec![family("app", &["app/.worktrees/login"], 3)]
        );
        assert_eq!(split.counts.focused, 1);
        assert_eq!(split.counts.outside, 3);
    }

    #[test]
    fn empty_focus_keeps_every_family_and_checkout() {
        let split = split_by_focus(&TreeFocus::parse(" , "), &workspace());
        assert_eq!(split.counts.focused, 4);
        assert_eq!(split.counts.total, 4);
        assert!(split.outside.is_empty());
        assert!(split.focused.iter().all(|f| !f.is_partial()));
    }

    #[test]
    fn counts_skip_hidden_repos_and_count_any_dirty_kind() {
        let staged = RepoSnapshot {
            has_staged: true,
            ..repo_snapshot("merger")
        };
        let untracked_linked = RepoSnapshot {
            has_untracked: true,
            ..linked("lib/.worktrees/spike", "lib", "spike")
        };
        let all = build_workspace_snapshot(
            &[
                on_branch("app", "feature/ABC-123-x"),
                repo("notes", true),
                staged,
                repo_snapshot("lib"),
                untracked_linked,
                repo_snapshot("services/api"),
                repo("vendor/big", true),
            ],
            &["vendor".to_string()],
            false,
            &[],
        );
        let visible = visible_workspace_snapshot(&all);
        let split = split_by_focus(&TreeFocus::parse("abc-123"), &visible);
        assert_eq!(
            split.counts,
            FocusCounts {
                focused: 1,
                total: 5,
                outside: 4,
                outside_dirty: 3,
            },
            "vendor/big is hidden: in neither count; lib is dirty through its linked worktree"
        );
        assert_eq!(split.outside, ["lib", "merger", "notes", "services/api"]);
    }

    #[test]
    fn feature_token_takes_the_first_ticket_like_match() {
        for (branch, token) in [
            ("feature/ABC-123-invoice-export", "ABC-123"),
            ("bugfix/web-42-y", "web-42"),
            ("feature/ab2c-7", "ab2c-7"),
            ("ABC-1", "ABC-1"),
            ("chore/x-y-PAY-9-z-10", "PAY-9"),
            ("feature/auth-refresh", "auth-refresh"),
            ("spike", "spike"),
            ("feature/a/b-c", "a/b-c"),
        ] {
            assert_eq!(
                feature_token(branch, None).as_deref(),
                Some(token),
                "{branch}"
            );
        }
    }

    #[test]
    fn feature_token_is_none_on_default_detached_or_unknown_head() {
        for branch in [
            "main",
            "master",
            "develop",
            "",
            "HEAD",
            "(detached)",
            "(unknown)",
        ] {
            assert_eq!(feature_token(branch, None), None, "{branch:?}");
        }
        assert_eq!(feature_token("trunk", Some("trunk")), None);
        assert_eq!(
            feature_token("main", Some("trunk")).as_deref(),
            Some("main"),
            "an override makes main a plain branch"
        );
    }

    #[test]
    fn repo_preset_is_the_family_primary_path() {
        assert_eq!(
            repo_preset(Some("app/.worktrees/feat-login"), Some("app")).as_deref(),
            Some("app")
        );
        assert_eq!(repo_preset(Some("lib"), None).as_deref(), Some("lib"));
        assert_eq!(repo_preset(None, None), None, "workspace and group rows");
    }

    #[test]
    fn match_ranges_are_case_insensitive_sorted_and_merged() {
        let focus = TreeFocus::parse("abc, c-1, zz");
        assert_eq!(
            match_ranges(&focus, "feature/ABC-123-abc"),
            vec![8..13, 16..19],
            "abc and c-1 touch and merge"
        );
        assert_eq!(match_ranges(&TreeFocus::parse("aa"), "aaa"), vec![0..3]);
        assert_eq!(match_ranges(&focus, "main"), Vec::<Range<usize>>::new());
        assert_eq!(
            match_ranges(&TreeFocus::default(), "abc"),
            Vec::<Range<usize>>::new()
        );
        let text = "Éclair-ÉCLAIR";
        let ranges = match_ranges(&TreeFocus::parse("éclair"), text);
        assert_eq!(ranges, vec![0..7, 8..15]);
        assert_eq!(&text[ranges[1].clone()], "ÉCLAIR");
    }
}

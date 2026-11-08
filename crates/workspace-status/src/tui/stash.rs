//! Stash menu (`S`): create / apply / pop / drop for the focused checkout.

use crate::git::StashTarget;
use crate::snapshot::{FileChange, WorkspaceSnapshot};

use super::tree::{NodeKind, VisibleRow};

/// Overlay op id (`s` / `a` / `p` / `D`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashOpId {
    Create,
    Apply,
    Pop,
    Drop,
}

/// One stash overlay row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashOp {
    pub id: StashOpId,
    pub key: char,
    pub label: &'static str,
    /// Stash entry of apply / pop / drop, picked when the menu opened.
    /// `None` for create.
    pub stash: Option<StashTarget>,
    pub paths: Option<Vec<String>>,
}

impl StashOp {
    /// `stash@{n}` of [`Self::stash`] as it was when the menu opened.
    pub fn stash_ref(&self) -> Option<&str> {
        self.stash.as_ref().map(|stash| stash.stash_ref.as_str())
    }
}

/// Focused-row facts used to list valid stash overlay ops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashOpsContext {
    pub dirty: bool,
    pub dirty_paths: Option<Vec<String>>,
    /// Graph stash row only. File/repo rows leave this empty.
    pub focused_stash: Option<StashTarget>,
    /// Latest stash for graph commit / uncommitted apply and pop.
    /// Drop never uses this — that still requires [`Self::focused_stash`].
    pub latest_stash: Option<StashTarget>,
}

/// Overlay key outcome while the stash menu is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StashMenuKeyResult {
    Cancel,
    Run(StashOp),
    Ignore,
}

/// Overlay ops valid for `ctx`, in create → apply → pop → drop order.
///
/// Apply / pop target the focused graph stash, else the latest stash.
/// Drop requires a focused graph stash row (never "drop latest").
/// A tree row that omits `latest_stash` stays create-only when dirty.
pub fn stash_ops_for_context(ctx: &StashOpsContext) -> Vec<StashOp> {
    let mut ops = Vec::new();
    if ctx.dirty {
        let paths = ctx.dirty_paths.clone().filter(|p| !p.is_empty());
        ops.push(StashOp {
            id: StashOpId::Create,
            key: 's',
            label: "stash",
            stash: None,
            paths,
        });
    }
    let apply_target = ctx.focused_stash.as_ref().or(ctx.latest_stash.as_ref());
    if let Some(stash) = apply_target {
        ops.push(StashOp {
            id: StashOpId::Apply,
            key: 'a',
            label: "apply stash",
            stash: Some(stash.clone()),
            paths: None,
        });
        ops.push(StashOp {
            id: StashOpId::Pop,
            key: 'p',
            label: "pop stash",
            stash: Some(stash.clone()),
            paths: None,
        });
    }
    if let Some(stash) = ctx.focused_stash.as_ref() {
        ops.push(StashOp {
            id: StashOpId::Drop,
            key: 'D',
            label: "drop stash",
            stash: Some(stash.clone()),
            paths: None,
        });
    }
    ops
}

/// Map overlay input to cancel / run / ignore. Enter runs the first listed op.
///
/// Drop is labelled `D` like the graph key; `d` runs it too.
pub fn resolve_stash_menu_key(
    input: Option<char>,
    enter: bool,
    escape: bool,
    ops: &[StashOp],
) -> StashMenuKeyResult {
    if escape {
        return StashMenuKeyResult::Cancel;
    }
    if enter {
        return match ops.first() {
            Some(op) => StashMenuKeyResult::Run(op.clone()),
            None => StashMenuKeyResult::Ignore,
        };
    }
    let Some(key) = input else {
        return StashMenuKeyResult::Ignore;
    };
    let key = if key == 'd' { 'D' } else { key };
    match ops.iter().find(|op| op.key == key) {
        Some(op) => StashMenuKeyResult::Run(op.clone()),
        None => StashMenuKeyResult::Ignore,
    }
}

/// Checkout path for stash / branch / push on the focused row.
pub fn checkout_path(row: &VisibleRow) -> Option<String> {
    match row.kind {
        NodeKind::Repo
        | NodeKind::Checkout
        | NodeKind::File
        | NodeKind::Dir
        | NodeKind::Section => row.repo.clone(),
        NodeKind::Workspace | NodeKind::Group => None,
    }
}

/// True when the focused row is a hidden checkout.
pub fn row_is_hidden(row: &VisibleRow, show_hidden: bool) -> bool {
    row.hidden && !show_hidden
}

/// Dirty flag and optional pathspecs for a stash create on `row`.
pub fn stash_dirty_for_row(
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
) -> (bool, Option<Vec<String>>) {
    match row.kind {
        NodeKind::File => {
            let Some(file) = row.file.as_ref() else {
                return (false, None);
            };
            if is_stashable(file) {
                (true, Some(vec![file.path.clone()]))
            } else {
                (false, None)
            }
        }
        NodeKind::Dir => {
            let Some(repo) = row.repo.as_deref() else {
                return (false, None);
            };
            let Some(dir) = crate::tui::tree::dir_path_from_id(&row.id, repo) else {
                return (false, None);
            };
            let Some(snap) = snapshot.repos.iter().find(|r| r.repo == repo) else {
                return (false, None);
            };
            let paths: Vec<String> = snap
                .changes
                .iter()
                .filter(|change| {
                    crate::tui::tree::path_under_dir(&change.path, &dir) && is_stashable(change)
                })
                .map(|change| change.path.clone())
                .collect();
            (!paths.is_empty(), Some(paths).filter(|p| !p.is_empty()))
        }
        NodeKind::Repo | NodeKind::Checkout => {
            let Some(repo) = row.repo.as_deref() else {
                return (false, None);
            };
            let Some(snap) = snapshot.repos.iter().find(|r| r.repo == repo) else {
                return (false, None);
            };
            let dirty = snap.has_unstaged || snap.has_staged || snap.has_untracked;
            (dirty, None)
        }
        NodeKind::Workspace | NodeKind::Group | NodeKind::Section => (false, None),
    }
}

fn is_stashable(change: &FileChange) -> bool {
    change.staged_status.is_some() || change.unstaged_status.is_some() || change.untracked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::stash_target;

    fn ctx(dirty: bool, focused: Option<&str>, latest: Option<&str>) -> StashOpsContext {
        StashOpsContext {
            dirty,
            dirty_paths: None,
            focused_stash: focused.map(stash_target),
            latest_stash: latest.map(stash_target),
        }
    }

    fn op_ids(ops: &[StashOp]) -> Vec<StashOpId> {
        ops.iter().map(|op| op.id).collect()
    }

    #[test]
    fn dirty_file_yields_create_with_paths() {
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty: true,
            dirty_paths: Some(vec!["src/a.rs".into()]),
            focused_stash: None,
            latest_stash: None,
        });
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].id, StashOpId::Create);
        assert_eq!(ops[0].key, 's');
        assert_eq!(
            ops[0].paths.as_deref(),
            Some(["src/a.rs".to_string()].as_slice())
        );
    }

    #[test]
    fn dirty_file_or_repo_is_create_only() {
        let ops = stash_ops_for_context(&ctx(true, None, None));
        assert_eq!(op_ids(&ops), vec![StashOpId::Create]);
    }

    #[test]
    fn clean_with_no_stash_is_empty() {
        let ops = stash_ops_for_context(&ctx(false, None, None));
        assert!(ops.is_empty());
    }

    #[test]
    fn latest_stash_offers_apply_pop_not_drop() {
        let ops = stash_ops_for_context(&ctx(false, None, Some("stash@{0}")));
        assert_eq!(op_ids(&ops), vec![StashOpId::Apply, StashOpId::Pop]);
        assert!(ops.iter().all(|op| op.stash_ref() == Some("stash@{0}")));
        assert!(!ops.iter().any(|op| op.id == StashOpId::Drop));
    }

    #[test]
    fn dirty_plus_latest_is_create_apply_pop() {
        let ops = stash_ops_for_context(&ctx(true, None, Some("stash@{2}")));
        assert_eq!(
            op_ids(&ops),
            vec![StashOpId::Create, StashOpId::Apply, StashOpId::Pop]
        );
        assert!(ops
            .iter()
            .filter(|op| op.id != StashOpId::Create)
            .all(|op| op.stash_ref() == Some("stash@{2}")));
    }

    #[test]
    fn focused_graph_stash_yields_apply_pop_drop() {
        let ops = stash_ops_for_context(&ctx(false, Some("stash@{1}"), Some("stash@{0}")));
        assert_eq!(
            op_ids(&ops),
            vec![StashOpId::Apply, StashOpId::Pop, StashOpId::Drop]
        );
        assert!(ops.iter().all(|op| op.stash_ref() == Some("stash@{1}")));
    }

    #[test]
    fn drop_requires_focused_stash_even_when_latest_exists() {
        let from_latest = stash_ops_for_context(&ctx(false, None, Some("stash@{0}")));
        assert!(!from_latest.iter().any(|op| op.id == StashOpId::Drop));
        let focused = stash_ops_for_context(&ctx(false, Some("stash@{1}"), Some("stash@{0}")));
        let drop = focused
            .iter()
            .find(|op| op.id == StashOpId::Drop)
            .expect("drop");
        assert_eq!(drop.stash_ref(), Some("stash@{1}"));
    }

    #[test]
    fn menu_keys_run_cancel_ignore() {
        let ops = stash_ops_for_context(&ctx(true, Some("stash@{0}"), None));
        assert!(matches!(
            resolve_stash_menu_key(None, false, true, &ops),
            StashMenuKeyResult::Cancel
        ));
        assert!(matches!(
            resolve_stash_menu_key(None, true, false, &ops),
            StashMenuKeyResult::Run(op) if op.id == StashOpId::Create
        ));
        assert!(matches!(
            resolve_stash_menu_key(Some('p'), false, false, &ops),
            StashMenuKeyResult::Run(op) if op.id == StashOpId::Pop
        ));
        assert!(matches!(
            resolve_stash_menu_key(Some('x'), false, false, &ops),
            StashMenuKeyResult::Ignore
        ));
    }

    #[test]
    fn drop_is_labelled_capital_d_and_takes_either_case() {
        let ops = stash_ops_for_context(&ctx(false, Some("stash@{1}"), None));
        let drop = ops
            .iter()
            .find(|op| op.id == StashOpId::Drop)
            .expect("drop");
        assert_eq!(drop.key, 'D');
        for key in ['d', 'D'] {
            assert!(matches!(
                resolve_stash_menu_key(Some(key), false, false, &ops),
                StashMenuKeyResult::Run(op) if op.id == StashOpId::Drop
            ));
        }
    }

    #[test]
    fn menu_lists_present_ops_with_their_keys() {
        let keys = |ops: Vec<StashOp>| ops.iter().map(|op| op.key).collect::<String>();
        let ops = stash_ops_for_context(&ctx(true, None, Some("stash@{0}")));
        assert_eq!(keys(ops), "sap");
        let drop_ops = stash_ops_for_context(&ctx(false, Some("stash@{1}"), None));
        assert_eq!(keys(drop_ops), "apD");
    }
}

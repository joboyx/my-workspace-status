//! Stash menu (`S`): create / apply / pop / drop for the focused checkout.

use crate::snapshot::{FileChange, WorkspaceSnapshot};

use super::tree::{NodeKind, VisibleRow};

/// Overlay op id (`s` / `a` / `p` / `d`).
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
    pub stash_ref: Option<String>,
    pub paths: Option<Vec<String>>,
}

/// Focused-row facts used to list valid stash overlay ops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashOpsContext {
    pub dirty: bool,
    pub dirty_paths: Option<Vec<String>>,
    /// Graph stash row only. File/repo rows leave this empty.
    pub focused_stash_ref: Option<String>,
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
/// A dirty file or repo is create-only. Apply / pop / drop require a focused
/// graph stash row. A clean repo is a no-op even when `stash@{0}` exists.
pub fn stash_ops_for_context(ctx: &StashOpsContext) -> Vec<StashOp> {
    let mut ops = Vec::new();
    if ctx.dirty {
        let paths = ctx.dirty_paths.clone().filter(|p| !p.is_empty());
        ops.push(StashOp {
            id: StashOpId::Create,
            key: 's',
            label: "stash",
            stash_ref: None,
            paths,
        });
    }
    if let Some(stash_ref) = ctx.focused_stash_ref.as_deref() {
        ops.push(StashOp {
            id: StashOpId::Apply,
            key: 'a',
            label: "apply stash",
            stash_ref: Some(stash_ref.to_string()),
            paths: None,
        });
        ops.push(StashOp {
            id: StashOpId::Pop,
            key: 'p',
            label: "pop stash",
            stash_ref: Some(stash_ref.to_string()),
            paths: None,
        });
        ops.push(StashOp {
            id: StashOpId::Drop,
            key: 'd',
            label: "drop stash",
            stash_ref: Some(stash_ref.to_string()),
            paths: None,
        });
    }
    ops
}

/// Map overlay input to cancel / run / ignore. Enter runs the first listed op.
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
    match ops.iter().find(|op| op.key == key) {
        Some(op) => StashMenuKeyResult::Run(op.clone()),
        None => StashMenuKeyResult::Ignore,
    }
}

/// Checkout path for stash / branch / push on the focused row.
pub fn checkout_path(row: &VisibleRow) -> Option<String> {
    match row.kind {
        NodeKind::Repo | NodeKind::Checkout | NodeKind::File => row.repo.clone(),
        NodeKind::Workspace | NodeKind::Group => None,
    }
}

/// True when the focused row is a hidden ignored checkout.
pub fn row_is_hidden_ignored(row: &VisibleRow, show_ignored: bool) -> bool {
    row.ignored && !show_ignored
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
        NodeKind::Workspace | NodeKind::Group => (false, None),
    }
}

fn is_stashable(change: &FileChange) -> bool {
    change.staged_status.is_some() || change.unstaged_status.is_some() || change.untracked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_file_yields_create_with_paths() {
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty: true,
            dirty_paths: Some(vec!["src/a.rs".into()]),
            focused_stash_ref: None,
        });
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].id, StashOpId::Create);
        assert_eq!(ops[0].key, 's');
        assert_eq!(ops[0].paths.as_deref(), Some(["src/a.rs".to_string()].as_slice()));
    }

    #[test]
    fn dirty_file_or_repo_is_create_only() {
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty: true,
            dirty_paths: None,
            focused_stash_ref: None,
        });
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create]
        );
    }

    #[test]
    fn clean_with_no_stash_is_empty() {
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty: false,
            dirty_paths: None,
            focused_stash_ref: None,
        });
        assert!(ops.is_empty());
    }

    #[test]
    fn clean_repo_with_latest_stash_is_empty() {
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty: false,
            dirty_paths: None,
            focused_stash_ref: None,
        });
        assert!(ops.is_empty());
    }

    #[test]
    fn focused_graph_stash_yields_apply_pop_drop() {
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty: false,
            dirty_paths: None,
            focused_stash_ref: Some("stash@{1}".into()),
        });
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Apply, StashOpId::Pop, StashOpId::Drop]
        );
        assert!(ops.iter().all(|op| op.stash_ref.as_deref() == Some("stash@{1}")));
    }

    #[test]
    fn menu_keys_run_cancel_ignore() {
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty: true,
            dirty_paths: None,
            focused_stash_ref: Some("stash@{0}".into()),
        });
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
}

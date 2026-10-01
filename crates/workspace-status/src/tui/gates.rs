//! Focus / depth / kind gates for tree writes.
//!
//! `dispatch` refuses workspace-tree writes when ViewStack depth ≥ 1 or when
//! the right pane is focused, unless the allow-list matches. The refusal
//! puts [`dispatch_noop_reason`] on the status line as a warn.

use super::action::Action;

/// Which list (or diff) the focused pane is driving.
///
/// Tree, graph list, commit-file list, or a focused file-diff row
/// (`None`). `j` / `k` and vertical wheel move that focused row. The
/// viewport keeps it near the vertical middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListFocusTarget {
    Tree,
    Graph,
    CommitFiles,
    None,
}

/// True when `action` is a tree write already refused at depth ≥ 1
/// (`s`/`u`/`x`/`f`/`p`/`P`/`d`/`b`/`W`). `S` is not in that set.
pub fn is_tree_write_blocked(action: &Action, depth: u8) -> bool {
    depth >= 1 && is_tree_write_action(action)
}

fn is_tree_write_action(action: &Action) -> bool {
    matches!(
        action,
        Action::Stage
            | Action::Unstage
            | Action::Revert
            | Action::Fetch
            | Action::Pull
            | Action::Push
            | Action::DefaultBranch
            | Action::Branch
            | Action::RemoveWorktree
    )
}

/// Actions that drive a list or row-scoped registry write.
/// Nav chrome, quit/help/refresh, theme/mouse,
/// view-mode toggles, file-diff row move, and overlay input are excluded.
pub fn is_left_list_action(action: &Action) -> bool {
    matches!(
        action,
        Action::Move(_)
            | Action::MoveToStart
            | Action::MoveToEnd
            | Action::FoldToggle
            | Action::FoldToggleSubtree
            | Action::FoldClose
            | Action::FoldOpen
            | Action::Stage
            | Action::Unstage
            | Action::Revert
            | Action::Edit
            | Action::ExternalDiff
            | Action::ToggleReviewed
            | Action::ToggleFullContext
            | Action::Branch
            | Action::RemoveWorktree
            | Action::GraphCheckout
            | Action::GraphCreateBranch
            | Action::GraphMerge
            | Action::GraphStashApply
            | Action::GraphStashDrop
            | Action::GraphStashPop
            | Action::StashMenu
            | Action::Fetch
            | Action::Pull
            | Action::Push
            | Action::DefaultBranch
    )
}

fn is_graph_write_action(action: &Action) -> bool {
    matches!(
        action,
        Action::GraphCheckout
            | Action::GraphCreateBranch
            | Action::GraphMerge
            | Action::GraphStashApply
            | Action::GraphStashDrop
            | Action::GraphStashPop
    )
}

fn is_move_action(action: &Action) -> bool {
    matches!(
        action,
        Action::Move(_) | Action::MoveToStart | Action::MoveToEnd
    )
}

fn is_fold_action(action: &Action) -> bool {
    matches!(
        action,
        Action::FoldToggle | Action::FoldToggleSubtree | Action::FoldClose | Action::FoldOpen
    )
}

fn is_commit_nav_action(action: &Action) -> bool {
    is_move_action(action)
        || is_fold_action(action)
        || matches!(
            action,
            Action::Edit | Action::ExternalDiff | Action::ToggleFullContext
        )
}

fn is_diff_file_write(action: &Action) -> bool {
    matches!(
        action,
        Action::Edit | Action::ExternalDiff | Action::ToggleFullContext | Action::ToggleReviewed
    )
}

/// True when a left-list action may still run with the right pane focused.
///
/// Allow-list: graph move/write
/// (`b`/`c`/`m`/`a`/`p`/`D` as `GraphCheckout` / `GraphCreateBranch` /
/// `GraphMerge` / stash apply/pop/drop), commit-file nav, diff move, and diff `e` /
/// `E` / Ctrl+O / space. Tree `b` (`Branch`) and `S` (`StashMenu`) stay left-only.
pub fn right_pane_left_list_allowed(target: ListFocusTarget, action: &Action) -> bool {
    let graph_move = target == ListFocusTarget::Graph && is_move_action(action);
    let graph_write = target == ListFocusTarget::Graph && is_graph_write_action(action);
    let commit_nav = target == ListFocusTarget::CommitFiles && is_commit_nav_action(action);
    let diff_move = target == ListFocusTarget::None && is_move_action(action);
    let diff_file_write = target == ListFocusTarget::None && is_diff_file_write(action);
    graph_move || graph_write || commit_nav || diff_move || diff_file_write
}

/// True when `dispatch` should swallow `action` as a no-op.
///
/// [`dispatch_noop_reason`] gives the status copy that says why.
pub fn dispatch_is_noop(
    action: &Action,
    depth: u8,
    focus_right: bool,
    target: ListFocusTarget,
) -> bool {
    if is_tree_write_blocked(action, depth) {
        return true;
    }
    focus_right && is_left_list_action(action) && !right_pane_left_list_allowed(target, action)
}

/// Why [`dispatch_is_noop`] swallows `action`, for the status line and the palette.
///
/// `None` when the action runs, and for list moves and folds, so a held
/// nav key cannot repaint a refusal on every repeat. Tree writes name the
/// key that gets back to the tree and what they would do there
/// (`focus the tree (Tab) to stage`).
pub fn dispatch_noop_reason(
    action: &Action,
    depth: u8,
    focus_right: bool,
    target: ListFocusTarget,
) -> Option<String> {
    if !dispatch_is_noop(action, depth, focus_right, target)
        || is_move_action(action)
        || is_fold_action(action)
    {
        return None;
    }
    // Esc pops a drill back to the tree; at depth 0 Tab switches panes.
    let back = if depth >= 1 { "Esc" } else { "Tab" };
    let reason = match action {
        Action::ToggleReviewed if depth >= 1 => REVIEWED_MARKS_ARE_FOR_TREE_FILES,
        Action::ToggleReviewed => FOCUS_A_FILE_TO_MARK_REVIEWED,
        Action::Edit => "focus a file to edit",
        Action::ExternalDiff => "focus a file to diff",
        Action::ToggleFullContext => "focus a file diff",
        Action::GraphCheckout | Action::GraphCreateBranch | Action::GraphMerge => {
            FOCUS_A_GRAPH_COMMIT
        }
        Action::GraphStashApply | Action::GraphStashPop | Action::GraphStashDrop => {
            FOCUS_A_GRAPH_STASH
        }
        Action::StashMenu if depth >= 2 => STASH_NEEDS_TREE_OR_GRAPH,
        // `S` runs from either left list (tree or a drill's graph).
        Action::StashMenu => "focus the left pane (Tab) to stash",
        _ => {
            return Some(format!(
                "focus the tree ({back}) to {}",
                tree_key_verb(action)
            ))
        }
    };
    Some(reason.into())
}

/// What a refused tree key would do, for [`dispatch_noop_reason`].
fn tree_key_verb(action: &Action) -> &'static str {
    match action {
        Action::Stage => "stage",
        Action::Unstage => "unstage",
        Action::Revert => "revert",
        Action::Fetch => "fetch",
        Action::Pull => "pull",
        Action::Push => "push",
        Action::DefaultBranch => "switch to the default branch",
        Action::Branch => "pick a branch",
        Action::RemoveWorktree => "remove a worktree",
        _ => "use this key",
    }
}

/// `S` (and the palette row) in a file-diff drill, where no list stashes.
pub const STASH_NEEDS_TREE_OR_GRAPH: &str = "focus the tree or graph (Esc) to stash";

/// Graph `b` / `c` / `m` (and their palette rows) off a graph commit row.
pub const FOCUS_A_GRAPH_COMMIT: &str = "focus a graph commit";

/// Graph `a` / `p` / `D` (and their palette rows) off a graph stash row.
pub const FOCUS_A_GRAPH_STASH: &str = "focus a graph stash row";

/// Space / palette copy when the focus is not a file row.
pub const FOCUS_A_FILE_TO_MARK_REVIEWED: &str = "focus a file to mark reviewed";

/// Space / palette copy in a commit drill: marks live on workspace-tree files.
pub const REVIEWED_MARKS_ARE_FOR_TREE_FILES: &str =
    "reviewed marks are for workspace-tree files · Esc back to the tree";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_reasons_name_the_way_back_and_the_action() {
        assert_eq!(
            dispatch_noop_reason(&Action::Stage, 0, true, ListFocusTarget::Graph).as_deref(),
            Some("focus the tree (Tab) to stage")
        );
        assert_eq!(
            dispatch_noop_reason(&Action::Pull, 1, false, ListFocusTarget::Graph).as_deref(),
            Some("focus the tree (Esc) to pull")
        );
        assert_eq!(
            dispatch_noop_reason(&Action::Branch, 0, true, ListFocusTarget::Graph).as_deref(),
            Some("focus the tree (Tab) to pick a branch")
        );
        assert_eq!(
            dispatch_noop_reason(&Action::StashMenu, 1, true, ListFocusTarget::CommitFiles)
                .as_deref(),
            Some("focus the left pane (Tab) to stash")
        );
        assert_eq!(
            dispatch_noop_reason(
                &Action::GraphCheckout,
                1,
                true,
                ListFocusTarget::CommitFiles
            )
            .as_deref(),
            Some(FOCUS_A_GRAPH_COMMIT)
        );
        assert_eq!(
            dispatch_noop_reason(&Action::StashMenu, 2, true, ListFocusTarget::None).as_deref(),
            Some(STASH_NEEDS_TREE_OR_GRAPH)
        );
        assert_eq!(
            dispatch_noop_reason(&Action::GraphStashPop, 0, true, ListFocusTarget::None).as_deref(),
            Some(FOCUS_A_GRAPH_STASH)
        );
        assert_eq!(
            dispatch_noop_reason(&Action::Move(1), 0, true, ListFocusTarget::Graph),
            None,
            "held nav keys never repaint a refusal"
        );
        assert_eq!(
            dispatch_noop_reason(&Action::Stage, 0, false, ListFocusTarget::Tree),
            None,
            "a tree key on the tree runs"
        );
    }

    #[test]
    fn tree_writes_blocked_at_depth_one_and_two() {
        for action in [
            Action::Stage,
            Action::Unstage,
            Action::Revert,
            Action::Fetch,
            Action::Pull,
            Action::Push,
            Action::DefaultBranch,
            Action::Branch,
            Action::RemoveWorktree,
        ] {
            assert!(!is_tree_write_blocked(&action, 0), "{action:?}");
            assert!(is_tree_write_blocked(&action, 1), "{action:?}");
            assert!(is_tree_write_blocked(&action, 2), "{action:?}");
        }
        assert!(!is_tree_write_blocked(&Action::StashMenu, 1));
        assert!(!is_tree_write_blocked(&Action::ToggleReviewed, 1));
        assert!(!is_tree_write_blocked(&Action::Edit, 1));
        assert!(!is_tree_write_blocked(&Action::ExternalDiff, 1));
    }

    #[test]
    fn right_pane_allow_list() {
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::GraphCheckout
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::GraphCreateBranch
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::GraphMerge
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::GraphStashApply
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::GraphStashPop
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::GraphStashDrop
        ));
        assert!(!right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::Branch
        ));
        assert!(!right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::StashMenu
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::CommitFiles,
            &Action::Move(1)
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::CommitFiles,
            &Action::Edit
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::CommitFiles,
            &Action::ExternalDiff
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::CommitFiles,
            &Action::ToggleFullContext
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::None,
            &Action::Edit
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::None,
            &Action::ExternalDiff
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::None,
            &Action::ToggleFullContext
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::None,
            &Action::ToggleReviewed
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::None,
            &Action::MoveToStart
        ));
        assert!(right_pane_left_list_allowed(
            ListFocusTarget::None,
            &Action::MoveToEnd
        ));
        assert!(!right_pane_left_list_allowed(
            ListFocusTarget::Graph,
            &Action::Stage
        ));
        assert!(!right_pane_left_list_allowed(
            ListFocusTarget::None,
            &Action::StashMenu
        ));
        assert!(!right_pane_left_list_allowed(
            ListFocusTarget::CommitFiles,
            &Action::Fetch
        ));
    }

    #[test]
    fn dispatch_noops_tree_writes_when_right_focused_at_depth_one() {
        for action in [
            Action::Stage,
            Action::Fetch,
            Action::Pull,
            Action::Push,
            Action::DefaultBranch,
            Action::Branch,
            Action::RemoveWorktree,
            Action::StashMenu,
        ] {
            assert!(
                dispatch_is_noop(&action, 1, true, ListFocusTarget::CommitFiles),
                "{action:?}"
            );
        }
        assert!(!dispatch_is_noop(
            &Action::Edit,
            1,
            true,
            ListFocusTarget::CommitFiles
        ));
        assert!(!dispatch_is_noop(
            &Action::ExternalDiff,
            1,
            true,
            ListFocusTarget::CommitFiles
        ));
        assert!(!dispatch_is_noop(
            &Action::ExternalDiff,
            0,
            true,
            ListFocusTarget::None
        ));
        assert!(!dispatch_is_noop(
            &Action::GraphCheckout,
            0,
            true,
            ListFocusTarget::Graph
        ));
        assert!(!dispatch_is_noop(
            &Action::GraphMerge,
            0,
            true,
            ListFocusTarget::Graph
        ));
        assert!(dispatch_is_noop(
            &Action::Branch,
            0,
            true,
            ListFocusTarget::Graph
        ));
        assert!(dispatch_is_noop(
            &Action::StashMenu,
            0,
            true,
            ListFocusTarget::Graph
        ));
        assert!(!dispatch_is_noop(
            &Action::ToggleFullContext,
            0,
            true,
            ListFocusTarget::None
        ));
        assert!(!dispatch_is_noop(
            &Action::ToggleReviewed,
            0,
            true,
            ListFocusTarget::None
        ));
        assert!(!dispatch_is_noop(
            &Action::MoveToStart,
            0,
            true,
            ListFocusTarget::None
        ));
        assert!(!dispatch_is_noop(
            &Action::MoveToEnd,
            0,
            true,
            ListFocusTarget::None
        ));
        assert!(dispatch_is_noop(
            &Action::ToggleReviewed,
            0,
            true,
            ListFocusTarget::Graph
        ));
    }
}

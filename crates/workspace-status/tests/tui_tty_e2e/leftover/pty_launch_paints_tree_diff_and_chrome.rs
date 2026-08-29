use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{documented_launch_first_paint, WAIT};

/// Spawn paints the documented first chrome. No keys.
///
/// Docs: left tree focused, first file selected, file diff on the right,
/// ignored repos hidden, No updates folded, breadcrumb is the workspace
/// basename while the right pane is a diff. Right-pane git is a worker, so
/// a tree-only frame or a `+dirty` substring is not enough. A no-op, a
/// blank screen, a graph-first launch, or a paint-changed-only assert
/// cannot pass.
#[test]
fn pty_launch_paints_tree_diff_and_chrome() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "documented first paint: focused tree, README cursor, file diff, breadcrumb, status, seed rows",
        WAIT,
    );
}

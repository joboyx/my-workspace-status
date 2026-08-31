//! Tree, file-diff, and graph hscroll oracles shared by TestBackend and PTY e2e.
//!
//! Tree callers pass **left-pane cells only**. A search chip that already
//! contains [`TREE_HSCROLL_TAIL`] must not count. Extraction stays
//! harness-specific: TestBackend clips by `pane_right_x`; PTY splits vt100
//! cells on the join. Graph / file-diff pan asserts the unique tail on the
//! right pane, not the tree or the search chip.

/// Visible prefix of the long tree path while the viewport is at column 0.
pub const TREE_HSCROLL_PREFIX: &str = "very-long";
/// Unique tail that appears only after the tree pans.
pub const TREE_HSCROLL_TAIL: &str = "TAIL99";
/// Directory under the daily `app` repo that holds [`TREE_HSCROLL_FILE`].
pub const TREE_HSCROLL_DIR: &str = "app/src/app/workspace-tree";
/// Long tree-row filename: clipped prefix plus [`TREE_HSCROLL_TAIL`].
pub const TREE_HSCROLL_FILE: &str = "very-long-workspace-tree-component-name-TAIL99.ts";
/// Unique tail of a long file-diff line.
pub const DIFF_HSCROLL_TAIL: &str = "UNIQUE_DIFF_TAIL";
/// Unique tail of a long graph subject (`n…` prefix plus this marker).
pub const GRAPH_HSCROLL_TAIL: &str = "UNIQUE_GRAPH_TAIL";
/// Prefix of [`GRAPH_HSCROLL_TAIL`] that still fits after max pan.
///
/// The remaining label viewport is often shorter than the full tail.
pub const GRAPH_HSCROLL_VISIBLE: &str = "UNIQUE_GRAP";

/// True when left-pane text shows the clipped prefix and not the tail.
pub fn is_clipped(left: &str) -> bool {
    left.contains(TREE_HSCROLL_PREFIX) && !left.contains(TREE_HSCROLL_TAIL)
}

/// True when left-pane text shows the tail and has dropped the prefix.
pub fn is_panned_to_tail(left: &str) -> bool {
    left.contains(TREE_HSCROLL_TAIL) && !left.contains(TREE_HSCROLL_PREFIX)
}

/// Assert the tree row is clipped: prefix on screen, tail off.
pub fn assert_clipped(left: &str) {
    assert!(
        is_clipped(left),
        "expected clipped tree row (`{TREE_HSCROLL_PREFIX}` without `{TREE_HSCROLL_TAIL}`):\n{left}"
    );
}

/// Assert the tree row has panned: tail on screen, prefix gone.
pub fn assert_panned_to_tail(left: &str) {
    assert!(
        is_panned_to_tail(left),
        "expected panned tree row (`{TREE_HSCROLL_TAIL}` without `{TREE_HSCROLL_PREFIX}`):\n{left}"
    );
}

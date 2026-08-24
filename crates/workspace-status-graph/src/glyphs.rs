//! Graph node, rail, and sync glyphs.
//!
//! Unicode matches `docs/git-graph-topology.md`.
//! ASCII matches `WS_STATUS_GLYPHS=ascii`. Topology is the same; only
//! the glyph map changes.

/// Columns occupied per logical lane (glyph + spacer / horizontal).
pub const CELL_W: usize = 2;

/// One paint mode for nodes, rails, and sync marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphSet {
    /// Regular commit node.
    pub commit: &'static str,
    /// HEAD commit node (including merge-at-HEAD).
    pub head_commit: &'static str,
    /// Uncommitted working-tree marker.
    pub uncommitted: &'static str,
    /// Stash side-leaf tip.
    pub stash: &'static str,
    /// Linked worktree marker. `ICON_LINKED_WORKTREE` (`` / `L`).
    pub worktree: &'static str,
    /// Ahead of upstream.
    pub ahead: &'static str,
    /// Behind upstream.
    pub behind: &'static str,
    /// Checkout mark on the named HEAD branch chip (`` / `+`).
    pub checkout_mark: &'static str,
    /// Synced local+remote mark inside a merged chip (`` / `=`).
    pub sync_mark: &'static str,
    /// Vertical through-rail.
    pub vertical: &'static str,
    /// Horizontal parent edge.
    pub horizontal: &'static str,
    /// Open lane to the right of the node (`╮` / `\`).
    pub corner_down_right: &'static str,
    /// Open lane to the left of the node (`╭` / `/`).
    pub corner_down_left: &'static str,
    /// Close lane into the node from the right (`╯` / `/`).
    pub corner_up_right: &'static str,
    /// Close lane into the node from the left (`╰` / `\`).
    pub corner_up_left: &'static str,
    /// Tee pointing left (`┤` / `+`).
    pub tee_left: &'static str,
    /// Tee pointing right (`├` / `+`).
    pub tee_right: &'static str,
    /// Tee pointing down (`┬` / `+`).
    pub tee_down: &'static str,
    /// Tee pointing up (`┴` / `+`).
    pub tee_up: &'static str,
    /// Four-way cross (`┼` / `+`).
    pub cross: &'static str,
}

/// Unicode glyphs used by the default widget.
pub const UNICODE: GlyphSet = GlyphSet {
    commit: "●",
    head_commit: "⊙",
    uncommitted: "○",
    stash: "◇",
    worktree: "",
    ahead: "↑",
    behind: "↓",
    checkout_mark: "",
    sync_mark: "",
    vertical: "│",
    horizontal: "─",
    corner_down_right: "╮",
    corner_down_left: "╭",
    corner_up_right: "╯",
    corner_up_left: "╰",
    tee_left: "┤",
    tee_right: "├",
    tee_down: "┬",
    tee_up: "┴",
    cross: "┼",
};

/// ASCII glyphs for terminals without the default font.
pub const ASCII: GlyphSet = GlyphSet {
    commit: "*",
    head_commit: "@",
    uncommitted: "o",
    stash: "s",
    worktree: "L",
    ahead: "^",
    behind: "v",
    checkout_mark: "+",
    sync_mark: "=",
    vertical: "|",
    horizontal: "-",
    corner_down_right: "\\",
    corner_down_left: "/",
    corner_up_right: "/",
    corner_up_left: "\\",
    tee_left: "+",
    tee_right: "+",
    tee_down: "+",
    tee_up: "+",
    cross: "+",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_glyph_is_linked_not_emoji() {
        assert_eq!(UNICODE.worktree, "");
        assert_eq!(ASCII.worktree, "L");
        assert_ne!(UNICODE.worktree, "🔗");
        assert_ne!(ASCII.worktree, "wt");
    }
}

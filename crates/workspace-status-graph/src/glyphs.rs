//! Graph node and sync glyphs.
//!
//! Unicode matches the Ink graph (`docs/git-graph-topology.md`).
//! ASCII matches `WS_STATUS_GLYPHS=ascii`.

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
    /// Linked worktree marker.
    pub worktree: &'static str,
    /// Ahead of upstream.
    pub ahead: &'static str,
    /// Behind upstream.
    pub behind: &'static str,
}

/// Unicode glyphs used by the default widget.
pub const UNICODE: GlyphSet = GlyphSet {
    commit: "●",
    head_commit: "⊙",
    uncommitted: "○",
    stash: "◇",
    worktree: "🔗",
    ahead: "↑",
    behind: "↓",
};

/// ASCII glyphs for terminals without the default font.
pub const ASCII: GlyphSet = GlyphSet {
    commit: "*",
    head_commit: "@",
    uncommitted: "o",
    stash: "s",
    worktree: "wt",
    ahead: "^",
    behind: "v",
};

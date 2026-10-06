//! Nerd Font glyph registry for the ratatui tree.
//!
//! A patched Nerd Font is a hard requirement. `WS_STATUS_GLYPHS=ascii` falls
//! back to plain markers. Every glyph occupies one terminal column: Nerd icons
//! live in the private-use area. Do not add emoji or CJK codepoints.
//!
//! [`ICON_CATALOG`] owns every tree and graph icon: both glyphs, a short
//! name, and a one-line meaning. The `icon_*` functions read it, and the
//! `?` help legend paints it. Graph glyphs come from
//! `workspace_status_graph::{UNICODE, ASCII}`.

use crate::helpers::visible_width;
use crate::snapshot::{FileChange, SyncStatus};

/// Pick the Nerd Font glyph unless ASCII fallback is active.
pub fn glyph(ascii: bool, nerd: &'static str, fallback: &'static str) -> &'static str {
    if ascii {
        fallback
    } else {
        nerd
    }
}

/* ── Structure ──────────────────────────────────────────────────────────── */

/// Fold chevron — expanded. Width 1 in both modes (not ASCII-gated).
pub const FOLD_EXPANDED: &str = "▾";
/// Fold chevron — collapsed.
pub const FOLD_COLLAPSED: &str = "▸";
/// ASCII fold expanded when `WS_STATUS_GLYPHS=ascii`.
pub const FOLD_EXPANDED_ASCII: &str = "v";
/// ASCII fold collapsed when `WS_STATUS_GLYPHS=ascii`.
pub const FOLD_COLLAPSED_ASCII: &str = ">";

/// Cursor accent bar painted in the left-most column of a focused list.
pub const CURSOR_BAR: &str = "▌";
/// Thinner selection marker on an unfocused list. Same column as [`CURSOR_BAR`].
pub const CURSOR_BAR_INACTIVE: &str = "▏";

/// Vertical rule between panes and inside the diff gutter.
#[allow(dead_code)]
pub const RULE: &str = "│";

/* ── Icon catalog ───────────────────────────────────────────────────────── */

/// Every icon and indicator the tree, the graph, and the `?` legend paint.
///
/// The discriminant is the row index in [`ICON_CATALOG`], so [`spec`] is
/// one array read. Add a variant and its catalog row together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    /// Workspace root row.
    Workspace,
    /// Primary checkout of a repository.
    Repo,
    /// Linked `git worktree` checkout (tree and graph).
    LinkedWorktree,
    /// Checked-out branch.
    Branch,
    /// HEAD is a strict ancestor of the default-branch tip.
    MergedIntoDefault,
    /// HEAD is not merged into the default branch.
    OpenVsDefault,
    /// Pull request open, not reported approved.
    PrOpen,
    /// Pull request open and approved.
    PrApproved,
    /// Pull request merged.
    PrMerged,
    /// Commits not pushed to upstream.
    Ahead,
    /// Upstream commits not pulled.
    Behind,
    /// Ahead and behind upstream.
    Diverged,
    /// Branch has no upstream.
    NoUpstream,
    /// No local changes and in sync.
    Clean,
    /// Up to date with upstream. Not painted today.
    Synced,
    /// `git status` failed for the checkout.
    StatusFailed,
    /// Repo listed as ignored in config.
    Ignored,
    /// Muted count of changed paths on a repo or checkout row.
    ChangeCount,
    /// Muted `N wt` count of checkouts on a repo family row.
    WorktreeCount,
    /// Staged section header.
    Staged,
    /// Unstaged changes section header.
    Changes,
    /// Folder row.
    Folder,
    /// File-type devicon ([`file_icon`] picks the glyph per path).
    FileType,
    /// Status letter `A`.
    StatusAdded,
    /// Status letter `S`.
    StatusStaged,
    /// Status letter `MS`.
    StatusStagedModified,
    /// Status letter `M`.
    StatusModified,
    /// Status letter `D`.
    StatusDeleted,
    /// Status letter `R`.
    StatusRenamed,
    /// Status letter `U`.
    StatusConflict,
    /// Status letter `C`.
    StatusCopied,
    /// File marked reviewed.
    Viewed,
    /// Open comment on the row.
    Comment,
    /// Every comment on the row is resolved.
    CommentResolved,
    /// Graph commit node.
    GraphCommit,
    /// Graph HEAD commit node.
    GraphHeadCommit,
    /// Graph stash tip.
    GraphStash,
    /// Graph uncommitted working-tree row.
    GraphUncommitted,
    /// Checkout mark inside the HEAD branch chip.
    ChipCheckout,
    /// Local and same-name remote on one commit, inside a branch chip.
    ChipSynced,
    /// Local branch chip.
    ChipLocal,
    /// Default branch chip.
    ChipDefault,
    /// Remote-tracking chip with no local match.
    ChipRemote,
    /// Tag chip.
    ChipTag,
    /// Detached `[HEAD]` chip.
    ChipDetachedHead,
    /// `[+N]` chip for refs that do not fit.
    ChipOverflow,
    /// Graph selection footer: more message lines below.
    GraphMoreBelow,
    /// Fold chevron, expanded.
    FoldExpanded,
    /// Fold chevron, collapsed.
    FoldCollapsed,
    /// Cursor bar on the focused list.
    CursorBar,
    /// Selection bar on an unfocused list.
    CursorBarInactive,
    /// Graph lane rails, corners, and tees.
    GraphRails,
    /// Help MOVE column title.
    HelpMove,
    /// Help VIEW column title.
    HelpView,
    /// Open folder. Not painted today.
    FolderOpen,
}

/// Where an icon sits in the `?` legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconGroup {
    /// Workspace tree rows.
    Tree,
    /// Graph pane rows and chips.
    Graph,
    /// Structure marks that carry no data.
    Chrome,
}

impl IconGroup {
    /// Legend heading for the group.
    pub fn title(self) -> &'static str {
        match self {
            Self::Tree => "Tree",
            Self::Graph => "Graph",
            Self::Chrome => "Chrome",
        }
    }
}

/// One catalog row: glyphs, a short name, and a one-line meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconSpec {
    /// The kind this row describes.
    pub kind: IconKind,
    /// Nerd Font (default) glyph or sample text.
    pub nerd: &'static str,
    /// `WS_STATUS_GLYPHS=ascii` glyph or sample text.
    pub ascii: &'static str,
    /// Short lower-case name.
    pub name: &'static str,
    /// One plain line for users: the legend row and a popover's first line.
    pub meaning: &'static str,
    /// Legend group. `None` keeps the row out of the `?` legend.
    pub group: Option<IconGroup>,
}

impl IconSpec {
    /// The glyph for the active glyph mode.
    pub fn glyph(&self, ascii: bool) -> &'static str {
        glyph(ascii, self.nerd, self.ascii)
    }
}

impl IconKind {
    /// Catalog row for this kind.
    pub fn spec(self) -> &'static IconSpec {
        spec(self)
    }

    /// The glyph for the active glyph mode.
    pub fn glyph(self, ascii: bool) -> &'static str {
        self.spec().glyph(ascii)
    }
}

/// Catalog row for `kind`.
pub fn spec(kind: IconKind) -> &'static IconSpec {
    let row = &ICON_CATALOG[kind as usize];
    debug_assert_eq!(row.kind, kind, "ICON_CATALOG order");
    row
}

const fn row(
    kind: IconKind,
    nerd: &'static str,
    ascii: &'static str,
    name: &'static str,
    meaning: &'static str,
    group: Option<IconGroup>,
) -> IconSpec {
    IconSpec {
        kind,
        nerd,
        ascii,
        name,
        meaning,
        group,
    }
}

const TREE: Option<IconGroup> = Some(IconGroup::Tree);
const GRAPH: Option<IconGroup> = Some(IconGroup::Graph);
const CHROME: Option<IconGroup> = Some(IconGroup::Chrome);

/// Graph rails sample for the legend: vertical, horizontal, the two
/// down corners, and the cross from [`workspace_status_graph::UNICODE`].
const GRAPH_RAILS_NERD: &str = "│─╮╭┼";
/// ASCII graph rails sample, same fields from [`workspace_status_graph::ASCII`].
const GRAPH_RAILS_ASCII: &str = "|-\\/+";

/// Every icon, in [`IconKind`] order. Graph glyphs come from the graph
/// crate's [`workspace_status_graph::GlyphSet`] (a test pins them).
pub const ICON_CATALOG: &[IconSpec] = {
    use workspace_status_graph::{ASCII as G_ASCII, UNICODE as G_NERD};
    use IconKind as K;
    &[
        row(
            K::Workspace,
            "\u{e5ff}",
            "#",
            "workspace",
            "Workspace root (current folder)",
            TREE,
        ),
        row(
            K::Repo,
            "\u{e702}",
            "@",
            "repo",
            "Git repository (primary checkout)",
            TREE,
        ),
        row(
            K::LinkedWorktree,
            G_NERD.worktree,
            G_ASCII.worktree,
            "worktree",
            "Linked git worktree checkout",
            TREE,
        ),
        row(
            K::Branch,
            "\u{e725}",
            "&",
            "branch",
            "Checked-out branch",
            TREE,
        ),
        row(
            K::MergedIntoDefault,
            "\u{f058}",
            "M",
            "merged",
            "HEAD is merged into the default branch",
            TREE,
        ),
        row(
            K::OpenVsDefault,
            "\u{f1bb}",
            "o",
            "not merged",
            "HEAD is not merged into the default branch yet",
            TREE,
        ),
        row(
            K::PrOpen,
            "\u{f407}",
            "P",
            "PR open",
            "Pull request is open",
            TREE,
        ),
        row(
            K::PrApproved,
            "\u{f42e}",
            "A",
            "PR approved",
            "Pull request is open and approved",
            TREE,
        ),
        row(
            K::PrMerged,
            "\u{f419}",
            "m",
            "PR merged",
            "Pull request is merged",
            TREE,
        ),
        row(
            K::Ahead,
            "\u{f062}",
            "^",
            "ahead",
            "Commits not pushed to upstream (+count)",
            TREE,
        ),
        row(
            K::Behind,
            "\u{f063}",
            "v",
            "behind",
            "Upstream commits not pulled (+count)",
            TREE,
        ),
        row(
            K::Diverged,
            "\u{e727}",
            "Y",
            "diverged",
            "Ahead and behind upstream",
            TREE,
        ),
        row(
            K::NoUpstream,
            "\u{f059}",
            "?",
            "no upstream",
            "Branch has no upstream",
            TREE,
        ),
        row(
            K::Clean,
            "\u{f00c}",
            ".",
            "clean",
            "In sync, no local changes; shown under No updates",
            TREE,
        ),
        // Same Nerd glyph as `Clean`; kept out of the legend while no row
        // paints it.
        row(
            K::Synced,
            "\u{f00c}",
            "=",
            "synced",
            "Up to date with upstream",
            None,
        ),
        row(
            K::StatusFailed,
            ICON_STATUS_FAILED_NERD,
            ICON_STATUS_FAILED_ASCII,
            "status failed",
            "git status failed for this checkout",
            TREE,
        ),
        row(
            K::Ignored,
            "\u{f070}",
            "~",
            "ignored",
            "Ignored in config; . shows or hides it",
            TREE,
        ),
        row(
            K::ChangeCount,
            "N",
            "N",
            "change count",
            "Paths with local changes (staged, unstaged, untracked)",
            TREE,
        ),
        row(
            K::WorktreeCount,
            "N wt",
            "N wt",
            "worktrees",
            "Checkouts in this repo family",
            TREE,
        ),
        row(K::Staged, "\u{f487}", "#", "staged", "Staged section", TREE),
        row(
            K::Changes,
            "\u{f040}",
            "~",
            "changes",
            "Unstaged changes section",
            TREE,
        ),
        row(K::Folder, "\u{f07b}", "/", "folder", "Folder", TREE),
        row(
            K::FileType,
            DEFAULT_FILE_GLYPH,
            ASCII_FILE_GLYPH,
            "file",
            "File type by name or extension",
            TREE,
        ),
        row(
            K::StatusAdded,
            "A",
            "A",
            "added",
            "New file (added or untracked)",
            TREE,
        ),
        row(
            K::StatusStaged,
            "S",
            "S",
            "staged file",
            "Change is staged, nothing unstaged",
            TREE,
        ),
        row(
            K::StatusStagedModified,
            "MS",
            "MS",
            "partly staged",
            "Staged, with more unstaged edits",
            TREE,
        ),
        row(
            K::StatusModified,
            "M",
            "M",
            "modified",
            "Modified, not staged",
            TREE,
        ),
        row(K::StatusDeleted, "D", "D", "deleted", "Deleted file", TREE),
        row(K::StatusRenamed, "R", "R", "renamed", "Renamed file", TREE),
        row(
            K::StatusConflict,
            "U",
            "U",
            "conflict",
            "Merge conflict to resolve",
            TREE,
        ),
        row(K::StatusCopied, "C", "C", "copied", "Copied file", TREE),
        row(
            K::Viewed,
            ICON_VIEWED_NERD,
            ICON_VIEWED_ASCII,
            "reviewed",
            "Marked reviewed (space)",
            TREE,
        ),
        row(
            K::Comment,
            ICON_COMMENT_NERD,
            ICON_COMMENT_ASCII,
            "comment",
            "Open comment on this row",
            TREE,
        ),
        row(
            K::CommentResolved,
            ICON_COMMENT_RESOLVED_NERD,
            ICON_COMMENT_RESOLVED_ASCII,
            "resolved",
            "All comments resolved",
            TREE,
        ),
        row(
            K::GraphCommit,
            G_NERD.commit,
            G_ASCII.commit,
            "commit",
            "Commit",
            GRAPH,
        ),
        row(
            K::GraphHeadCommit,
            G_NERD.head_commit,
            G_ASCII.head_commit,
            "HEAD commit",
            "HEAD commit",
            GRAPH,
        ),
        row(
            K::GraphStash,
            G_NERD.stash,
            G_ASCII.stash,
            "stash",
            "Stash (one-node side-branch tip)",
            GRAPH,
        ),
        row(
            K::GraphUncommitted,
            G_NERD.uncommitted,
            G_ASCII.uncommitted,
            "uncommitted",
            "Working-tree changes row",
            GRAPH,
        ),
        row(
            K::ChipCheckout,
            G_NERD.checkout_mark,
            G_ASCII.checkout_mark,
            "checked out",
            "HEAD is on this branch (head colour)",
            GRAPH,
        ),
        row(
            K::ChipSynced,
            G_NERD.sync_mark,
            G_ASCII.sync_mark,
            "same commit",
            "Local and same-name remote here (remote colour alone)",
            GRAPH,
        ),
        row(
            K::ChipLocal,
            "[name]",
            "[name]",
            "local",
            "Local branch",
            GRAPH,
        ),
        row(
            K::ChipDefault,
            "[main]",
            "[main]",
            "default",
            "Default branch (local or remote)",
            GRAPH,
        ),
        row(
            K::ChipRemote,
            "[name]",
            "[name]",
            "remote",
            "Remote branch with no local match",
            GRAPH,
        ),
        row(K::ChipTag, "[name]", "[name]", "tag", "Tag", GRAPH),
        row(
            K::ChipDetachedHead,
            "[HEAD]",
            "[HEAD]",
            "detached",
            "HEAD is not on a branch",
            GRAPH,
        ),
        row(
            K::ChipOverflow,
            "[+N]",
            "[+N]",
            "more refs",
            "N more refs that do not fit",
            GRAPH,
        ),
        row(
            K::GraphMoreBelow,
            G_NERD.more_below,
            G_ASCII.more_below,
            "more lines",
            "More commit message lines below (count)",
            GRAPH,
        ),
        row(
            K::FoldExpanded,
            FOLD_EXPANDED,
            FOLD_EXPANDED_ASCII,
            "expanded",
            "Open row; h or z folds it",
            CHROME,
        ),
        row(
            K::FoldCollapsed,
            FOLD_COLLAPSED,
            FOLD_COLLAPSED_ASCII,
            "collapsed",
            "Folded row; l or z opens it",
            CHROME,
        ),
        row(
            K::CursorBar,
            CURSOR_BAR,
            CURSOR_BAR,
            "cursor",
            "Cursor row in the focused pane",
            CHROME,
        ),
        row(
            K::CursorBarInactive,
            CURSOR_BAR_INACTIVE,
            CURSOR_BAR_INACTIVE,
            "selection",
            "Selected row in the other pane",
            CHROME,
        ),
        row(
            K::GraphRails,
            GRAPH_RAILS_NERD,
            GRAPH_RAILS_ASCII,
            "rails",
            "Graph lanes from commits to parents",
            CHROME,
        ),
        row(
            K::HelpMove,
            "\u{e7a2}",
            "+",
            "move",
            "Help MOVE column",
            None,
        ),
        row(
            K::HelpView,
            "\u{f440}",
            "%",
            "view",
            "Help VIEW column",
            None,
        ),
        row(
            K::FolderOpen,
            "\u{f07c}",
            "/",
            "open folder",
            "Open folder",
            None,
        ),
    ]
};

pub fn icon_workspace(ascii: bool) -> &'static str {
    IconKind::Workspace.glyph(ascii)
}
pub fn icon_repo(ascii: bool) -> &'static str {
    IconKind::Repo.glyph(ascii)
}
/// Linked `git worktree` checkout. Nerd: nf-oct-link; ASCII: `L`.
pub fn icon_linked_worktree(ascii: bool) -> &'static str {
    IconKind::LinkedWorktree.glyph(ascii)
}
pub fn icon_branch(ascii: bool) -> &'static str {
    IconKind::Branch.glyph(ascii)
}
/// Help MOVE column. Nerd: nf-dev-terminal_badge; ASCII: `+`.
pub fn icon_move(ascii: bool) -> &'static str {
    IconKind::HelpMove.glyph(ascii)
}
/// Help VIEW column. Nerd: nf-oct-diff; ASCII: `%`.
pub fn icon_diff(ascii: bool) -> &'static str {
    IconKind::HelpView.glyph(ascii)
}
/// Font named in the help footer (`REQUIRED_FONT`).
pub const REQUIRED_FONT: &str = "MesloLGS NF";
/// Staged section header. Nerd: nf-oct-package (`U+F487`); ASCII: `#`.
pub fn icon_staged(ascii: bool) -> &'static str {
    IconKind::Staged.glyph(ascii)
}
/// Changes section header. Nerd: nf-fa-pencil (`U+F040`); ASCII: `~`.
pub fn icon_changes(ascii: bool) -> &'static str {
    IconKind::Changes.glyph(ascii)
}
pub fn icon_folder(ascii: bool) -> &'static str {
    IconKind::Folder.glyph(ascii)
}
#[allow(dead_code)]
pub fn icon_folder_open(ascii: bool) -> &'static str {
    IconKind::FolderOpen.glyph(ascii)
}
pub fn icon_clean(ascii: bool) -> &'static str {
    IconKind::Clean.glyph(ascii)
}
pub fn icon_ignored(ascii: bool) -> &'static str {
    IconKind::Ignored.glyph(ascii)
}
pub fn icon_ahead(ascii: bool) -> &'static str {
    IconKind::Ahead.glyph(ascii)
}
pub fn icon_behind(ascii: bool) -> &'static str {
    IconKind::Behind.glyph(ascii)
}
pub fn icon_diverged(ascii: bool) -> &'static str {
    IconKind::Diverged.glyph(ascii)
}
pub fn icon_no_upstream(ascii: bool) -> &'static str {
    IconKind::NoUpstream.glyph(ascii)
}
pub fn icon_synced(ascii: bool) -> &'static str {
    IconKind::Synced.glyph(ascii)
}
/// HEAD is a strict ancestor of the default-branch tip. Nerd: nf-fa-check-circle; ASCII: `M`.
///
/// Same-commit as that tip is not merged (`icon_open_vs_default`).
pub fn icon_merged_into_default(ascii: bool) -> &'static str {
    IconKind::MergedIntoDefault.glyph(ascii)
}
/// HEAD is not merged into default. Nerd: nf-fa-tree; ASCII: `o`.
///
/// Includes a just-created branch whose HEAD matches the default tip.
pub fn icon_open_vs_default(ascii: bool) -> &'static str {
    IconKind::OpenVsDefault.glyph(ascii)
}
/// PR badge: open, not reported approved. Nerd: nf-oct-git_pull_request
/// (`U+F407`); ASCII: `P`.
pub fn icon_pr_open(ascii: bool) -> &'static str {
    IconKind::PrOpen.glyph(ascii)
}
/// PR badge: open and approved. Nerd: nf-oct-check (`U+F42E`); ASCII: `A`.
///
/// Distinct from [`icon_merged_into_default`].
pub fn icon_pr_approved(ascii: bool) -> &'static str {
    IconKind::PrApproved.glyph(ascii)
}
/// PR badge: merged. Nerd: nf-oct-git_merge (`U+F419`); ASCII: `m`.
pub fn icon_pr_merged(ascii: bool) -> &'static str {
    IconKind::PrMerged.glyph(ascii)
}
/// `ICON_STATUS_FAILED` nerd glyph: nf-fa-warning (`U+F071`).
pub const ICON_STATUS_FAILED_NERD: &str = "\u{f071}";
/// `ICON_STATUS_FAILED` ASCII fallback.
pub const ICON_STATUS_FAILED_ASCII: &str = "!";

/// Repo row whose `git status` failed. Nerd: nf-fa-warning; ASCII: `!`.
/// Paints in place of the sync mark, which would otherwise read as no upstream.
pub fn icon_status_failed(ascii: bool) -> &'static str {
    IconKind::StatusFailed.glyph(ascii)
}

/// `ICON_VIEWED` nerd glyph: nf-fa-eye (`U+F06E`).
///
/// Do not substitute `◉` or another PUA eye.
pub const ICON_VIEWED_NERD: &str = "\u{f06e}";
/// `ICON_VIEWED` ASCII fallback.
pub const ICON_VIEWED_ASCII: &str = "*";

/// Reviewed mark on a dirty file row. Nerd: nf-fa-eye (`U+F06E`); ASCII: `*`.
/// Distinct from `ICON_CLEAN` / `ICON_SYNCED`.
pub fn icon_viewed(ascii: bool) -> &'static str {
    IconKind::Viewed.glyph(ascii)
}

/// `ICON_COMMENT` nerd glyph: nf-fa-comment (`U+F075`).
pub const ICON_COMMENT_NERD: &str = "\u{f075}";
/// `ICON_COMMENT` ASCII fallback.
pub const ICON_COMMENT_ASCII: &str = "\"";
/// Resolved comment nerd glyph: nf-fa-comment-o (`U+F0E5`).
pub const ICON_COMMENT_RESOLVED_NERD: &str = "\u{f0e5}";
/// Resolved comment ASCII fallback. Distinct from open `"`.
pub const ICON_COMMENT_RESOLVED_ASCII: &str = "'";

/// Comment mark on a row or diff line. Nerd: nf-fa-comment; ASCII: `"`.
pub fn icon_comment(ascii: bool) -> &'static str {
    IconKind::Comment.glyph(ascii)
}

/// Resolved comment mark. Nerd: nf-fa-comment-o; ASCII: `'`.
pub fn icon_comment_resolved(ascii: bool) -> &'static str {
    IconKind::CommentResolved.glyph(ascii)
}

/// Display columns reserved for the comment mark on every numbered diff cell.
///
/// Always reserved so adding a comment does not shift line numbers. Both
/// Nerd and ASCII marks are one column (same contract as the rest of this
/// registry). Themes change colour, not width.
pub fn comment_mark_cols(ascii: bool) -> usize {
    visible_width(icon_comment(ascii))
        .max(visible_width(icon_comment_resolved(ascii)))
        .max(1)
}

const DEFAULT_FILE_GLYPH: &str = "";
const ASCII_FILE_GLYPH: &str = "·";

/// Devicon for a repo-relative file path. Exact filename wins, then extension.
pub fn file_icon(ascii: bool, file_path: &str) -> FileIcon {
    if ascii {
        return FileIcon {
            glyph: ASCII_FILE_GLYPH,
            color: None,
        };
    }
    let name = file_path
        .rsplit('/')
        .next()
        .unwrap_or(file_path)
        .to_ascii_lowercase();
    if let Some(icon) = filename_icon(&name) {
        return icon;
    }
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if let Some(icon) = extension_icon(ext) {
        return icon;
    }
    FileIcon {
        glyph: DEFAULT_FILE_GLYPH,
        color: None,
    }
}

/// File type a popover names for `file_path`: the file name as written
/// when a filename rule picks its devicon (`package.json`, `README.md`),
/// else the lower-case extension (`rs`), else `plain`. Same lookup order
/// as [`file_icon`], which matches names case-insensitively.
pub fn file_type_name(file_path: &str) -> String {
    let base = file_path.rsplit('/').next().unwrap_or(file_path);
    let name = base.to_ascii_lowercase();
    if filename_icon(&name).is_some() {
        return base.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_string(),
        _ => "plain".into(),
    }
}

/// Nerd file glyph plus optional hex colour (theme `file` when `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIcon {
    pub glyph: &'static str,
    pub color: Option<&'static str>,
}

fn filename_icon(name: &str) -> Option<FileIcon> {
    Some(match name {
        ".gitignore" | ".gitattributes" | ".gitmodules" => FileIcon {
            glyph: "",
            color: Some("#e24329"),
        },
        "package.json" => FileIcon {
            glyph: "",
            color: Some("#e8274b"),
        },
        "package-lock.json" => FileIcon {
            glyph: "",
            color: Some("#7a0d21"),
        },
        "dockerfile" => FileIcon {
            glyph: "",
            color: Some("#458ee6"),
        },
        "makefile" => FileIcon {
            glyph: "",
            color: Some("#6d8086"),
        },
        "readme.md" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        ".envrc" | ".env" => FileIcon {
            glyph: "",
            color: Some("#faf743"),
        },
        _ => return None,
    })
}

fn extension_icon(ext: &str) -> Option<FileIcon> {
    Some(match ext {
        "ts" | "mts" | "cts" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "tsx" | "jsx" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "js" | "mjs" | "cjs" => FileIcon {
            glyph: "",
            color: Some("#cbcb41"),
        },
        "json" => FileIcon {
            glyph: "",
            color: Some("#cbcb41"),
        },
        "md" | "mdx" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "py" => FileIcon {
            glyph: "",
            color: Some("#ffbc03"),
        },
        "sh" | "bash" | "zsh" => FileIcon {
            glyph: "",
            color: Some("#89e051"),
        },
        "html" => FileIcon {
            glyph: "",
            color: Some("#e34c26"),
        },
        "css" => FileIcon {
            glyph: "",
            color: Some("#563d7c"),
        },
        "scss" => FileIcon {
            glyph: "",
            color: Some("#f55385"),
        },
        "yml" | "yaml" | "toml" => FileIcon {
            glyph: "",
            color: Some("#6d8086"),
        },
        "java" => FileIcon {
            glyph: "",
            color: Some("#cc3e44"),
        },
        "cs" => FileIcon {
            glyph: "",
            color: Some("#596706"),
        },
        "go" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "rs" => FileIcon {
            glyph: "",
            color: Some("#dea584"),
        },
        "sql" => FileIcon {
            glyph: "",
            color: Some("#dad8d8"),
        },
        "png" | "jpg" | "jpeg" | "gif" => FileIcon {
            glyph: "",
            color: Some("#a074c4"),
        },
        "svg" => FileIcon {
            glyph: "",
            color: Some("#ffb13b"),
        },
        "lock" => FileIcon {
            glyph: "",
            color: Some("#bbbbbb"),
        },
        "txt" => FileIcon {
            glyph: "",
            color: None,
        },
        _ => return None,
    })
}

/* ── File status ────────────────────────────────────────────────────────── */

/// Letter code aligned with `FileStatusLetter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatusLetter {
    A,
    M,
    S,
    Ms,
    D,
    R,
    U,
    C,
}

impl FileStatusLetter {
    /// Letter token (`A` / `S` / `MS` / `M` / `D` / `R` / `U` / `C`).
    ///
    /// Watch signatures and tests use this, not the 2-column badge.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::M => "M",
            Self::S => "S",
            Self::Ms => "MS",
            Self::D => "D",
            Self::R => "R",
            Self::U => "U",
            Self::C => "C",
        }
    }

    /// Exactly 2 display columns — right-aligned like the VS Code SCM gutter.
    pub fn badge(self) -> &'static str {
        match self {
            Self::A => "A ",
            Self::M => "M ",
            Self::S => "S ",
            Self::Ms => "MS",
            Self::D => "D ",
            Self::R => "R ",
            Self::U => "U ",
            Self::C => "C ",
        }
    }

    /// Catalog kind of this letter's badge.
    pub fn icon_kind(self) -> IconKind {
        match self {
            Self::A => IconKind::StatusAdded,
            Self::M => IconKind::StatusModified,
            Self::S => IconKind::StatusStaged,
            Self::Ms => IconKind::StatusStagedModified,
            Self::D => IconKind::StatusDeleted,
            Self::R => IconKind::StatusRenamed,
            Self::U => IconKind::StatusConflict,
            Self::C => IconKind::StatusCopied,
        }
    }

    /// `statusColor` token name.
    pub fn color_role(self) -> StatusColorRole {
        match self {
            Self::A | Self::S => StatusColorRole::Added,
            Self::M | Self::Ms => StatusColorRole::Modified,
            Self::D | Self::U => StatusColorRole::Deleted,
            Self::R | Self::C => StatusColorRole::Renamed,
        }
    }
}

/// Semantic colour for a status letter / sync mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusColorRole {
    Added,
    Modified,
    Deleted,
    Renamed,
    Muted,
    #[allow(dead_code)]
    File,
}

/// Map a FileChange to the same letter vocabulary as `statusLetterFromChange`.
pub fn status_letter_from_change(change: &FileChange) -> FileStatusLetter {
    // Conflict before MS — staged+unstaged both set must not swallow U.
    let unstaged = change.unstaged_status.as_deref();
    let staged = change.staged_status.as_deref();
    if unstaged == Some("U") || staged == Some("U") {
        return FileStatusLetter::U;
    }
    if staged.is_some() && unstaged.is_some() {
        return FileStatusLetter::Ms;
    }
    let status = unstaged.or(staged);
    if status == Some("R") {
        return FileStatusLetter::R;
    }
    if status == Some("D") {
        return FileStatusLetter::D;
    }
    if change.untracked || status == Some("A") {
        return FileStatusLetter::A;
    }
    if staged.is_some() && unstaged.is_none() {
        return FileStatusLetter::S;
    }
    if status == Some("C") {
        return FileStatusLetter::C;
    }
    FileStatusLetter::M
}

/// Exactly 2 display columns for a file-change badge.
pub fn tui_file_badge(change: &FileChange) -> &'static str {
    status_letter_from_change(change).badge()
}

/* ── Branch / sync ──────────────────────────────────────────────────────── */

/// Merge-into-default mark for TUI branch chrome. Never emoji.
///
/// `Some(true)` is the checkmark. `Some(false)` is open, including a
/// just-created branch whose HEAD matches the default tip.
pub fn tui_merge_mark(ascii: bool, merged: Option<bool>) -> &'static str {
    match merged {
        Some(true) => icon_merged_into_default(ascii),
        Some(false) => icon_open_vs_default(ascii),
        None => "",
    }
}

/// Sync mark: glyph plus commit count, e.g. ahead-by-2 or behind-by-3.
pub fn tui_sync_mark(ascii: bool, status: SyncStatus, note: &str) -> String {
    match status {
        SyncStatus::NoUpstream => icon_no_upstream(ascii).to_string(),
        SyncStatus::Behind => {
            let count = capture_count(note, "behind by ");
            format!("{}{count}", icon_behind(ascii))
        }
        SyncStatus::Ahead => {
            let count = capture_count(note, "ahead by ");
            format!("{}{count}", icon_ahead(ascii))
        }
        SyncStatus::Diverged => icon_diverged(ascii).to_string(),
        SyncStatus::UpToDate => icon_synced(ascii).to_string(),
    }
}

/// The digits right after `prefix` in a sync note (`ahead by 3 commits`
/// gives `3`), or empty.
pub(crate) fn capture_count<'a>(note: &'a str, prefix: &str) -> &'a str {
    let Some(idx) = note.find(prefix) else {
        return "";
    };
    let start = idx + prefix.len();
    let end = note[start..]
        .find(|c: char| !c.is_ascii_digit())
        .map(|n| start + n)
        .unwrap_or(note.len());
    &note[start..end]
}

/// Colour matching `tui_sync_mark` semantics.
pub fn sync_color_role(status: SyncStatus) -> StatusColorRole {
    match status {
        SyncStatus::Behind => StatusColorRole::Deleted,
        SyncStatus::Ahead => StatusColorRole::Added,
        SyncStatus::Diverged => StatusColorRole::Modified,
        SyncStatus::UpToDate | SyncStatus::NoUpstream => StatusColorRole::Muted,
    }
}

/// Truncate a string to at most `width` terminal columns.
pub fn truncate_visible(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if visible_width(value) <= width {
        return value.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in value.chars() {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let cw = visible_width(s);
        if w + cw > width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// True if any codepoint is in the emoji / pictograph range (≥ U+1F300).
#[allow(dead_code)]
pub fn has_wide_emoji(value: &str) -> bool {
    value.chars().any(|ch| (ch as u32) >= 0x1f300)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(untracked: bool, staged: Option<&str>, unstaged: Option<&str>) -> FileChange {
        FileChange {
            path: "a".into(),
            staged_status: staged.map(str::to_string),
            unstaged_status: unstaged.map(str::to_string),
            untracked,
            old_path: None,
        }
    }

    #[test]
    fn badges_are_two_columns() {
        let letters = [
            FileStatusLetter::A,
            FileStatusLetter::M,
            FileStatusLetter::S,
            FileStatusLetter::Ms,
            FileStatusLetter::D,
            FileStatusLetter::R,
            FileStatusLetter::U,
            FileStatusLetter::C,
        ];
        let expected = ["A ", "M ", "S ", "MS", "D ", "R ", "U ", "C "];
        for (letter, want) in letters.into_iter().zip(expected) {
            assert_eq!(letter.badge(), want);
            assert_eq!(visible_width(letter.badge()), 2);
        }
        assert_eq!(
            status_letter_from_change(&change(true, None, None)),
            FileStatusLetter::A
        );
        assert_eq!(
            status_letter_from_change(&change(false, Some("M"), Some("M"))),
            FileStatusLetter::Ms
        );
        assert_eq!(
            status_letter_from_change(&change(false, Some("M"), None)),
            FileStatusLetter::S
        );
        assert_eq!(
            status_letter_from_change(&change(false, None, Some("M"))),
            FileStatusLetter::M
        );
        assert_eq!(
            status_letter_from_change(&change(false, Some("U"), Some("M"))),
            FileStatusLetter::U
        );
        assert_eq!(tui_file_badge(&change(true, None, None)), "A ");
    }

    #[test]
    fn structure_glyphs_are_width_one() {
        let ascii = [
            icon_workspace(true),
            icon_repo(true),
            icon_linked_worktree(true),
            icon_branch(true),
            icon_folder(true),
            icon_clean(true),
            icon_ignored(true),
            icon_ahead(true),
            icon_behind(true),
            icon_diverged(true),
            icon_no_upstream(true),
            icon_synced(true),
            icon_status_failed(true),
            icon_merged_into_default(true),
            icon_open_vs_default(true),
            icon_viewed(true),
            icon_comment(true),
            icon_comment_resolved(true),
            icon_staged(true),
            icon_changes(true),
            icon_pr_open(true),
            icon_pr_approved(true),
            icon_pr_merged(true),
            ASCII_FILE_GLYPH,
        ];
        let nerd = [
            icon_workspace(false),
            icon_repo(false),
            icon_linked_worktree(false),
            icon_branch(false),
            icon_folder(false),
            icon_clean(false),
            icon_ignored(false),
            icon_ahead(false),
            icon_behind(false),
            icon_diverged(false),
            icon_no_upstream(false),
            icon_synced(false),
            icon_status_failed(false),
            icon_merged_into_default(false),
            icon_open_vs_default(false),
            icon_viewed(false),
            icon_comment(false),
            icon_comment_resolved(false),
            icon_staged(false),
            icon_changes(false),
            icon_pr_open(false),
            icon_pr_approved(false),
            icon_pr_merged(false),
            CURSOR_BAR,
            CURSOR_BAR_INACTIVE,
            FOLD_EXPANDED,
            FOLD_COLLAPSED,
            RULE,
        ];
        for g in ascii.into_iter().chain(nerd) {
            assert_eq!(visible_width(g), 1, "{g:?}");
            assert!(!has_wide_emoji(g), "{g:?}");
        }
        assert_eq!(icon_workspace(true), "#");
        assert_eq!(icon_staged(true), "#");
        assert_eq!(icon_changes(true), "~");
        assert_eq!(icon_staged(false), "\u{f487}");
        assert_eq!(icon_changes(false), "\u{f040}");
        assert_eq!(icon_viewed(true), "*");
        assert_eq!(icon_comment(true), "\"");
        assert_eq!(icon_comment(false), ICON_COMMENT_NERD);
        assert_eq!(icon_comment_resolved(true), "'");
        assert_eq!(icon_comment_resolved(false), ICON_COMMENT_RESOLVED_NERD);
        assert_eq!(visible_width(icon_comment(false)), 1);
        assert_eq!(visible_width(icon_comment_resolved(false)), 1);
        assert_eq!(comment_mark_cols(true), 1);
        assert_eq!(comment_mark_cols(false), 1);
        assert_ne!(icon_viewed(false), icon_clean(false));
        assert_ne!(icon_viewed(false), icon_synced(false));
    }

    /// The discriminant indexes the catalog, and every kind has a row.
    #[test]
    fn catalog_rows_follow_kind_order() {
        assert_eq!(ICON_CATALOG.len(), IconKind::FolderOpen as usize + 1);
        for (i, row) in ICON_CATALOG.iter().enumerate() {
            assert_eq!(row.kind as usize, i, "{row:?}");
            assert_eq!(spec(row.kind), row);
            assert_eq!(row.kind.spec(), row);
        }
    }

    /// The `icon_*` functions return the catalog glyph, and no glyph moved
    /// when the catalog took them over.
    #[test]
    fn icon_functions_read_the_catalog() {
        type IconFn = fn(bool) -> &'static str;
        let fns: [(IconFn, IconKind, &str, &str); 23] = [
            (icon_workspace, IconKind::Workspace, "\u{e5ff}", "#"),
            (icon_repo, IconKind::Repo, "\u{e702}", "@"),
            (
                icon_linked_worktree,
                IconKind::LinkedWorktree,
                "\u{f481}",
                "L",
            ),
            (icon_branch, IconKind::Branch, "\u{e725}", "&"),
            (icon_move, IconKind::HelpMove, "\u{e7a2}", "+"),
            (icon_diff, IconKind::HelpView, "\u{f440}", "%"),
            (icon_staged, IconKind::Staged, "\u{f487}", "#"),
            (icon_changes, IconKind::Changes, "\u{f040}", "~"),
            (icon_folder, IconKind::Folder, "\u{f07b}", "/"),
            (icon_folder_open, IconKind::FolderOpen, "\u{f07c}", "/"),
            (icon_clean, IconKind::Clean, "\u{f00c}", "."),
            (icon_ignored, IconKind::Ignored, "\u{f070}", "~"),
            (icon_ahead, IconKind::Ahead, "\u{f062}", "^"),
            (icon_behind, IconKind::Behind, "\u{f063}", "v"),
            (icon_diverged, IconKind::Diverged, "\u{e727}", "Y"),
            (icon_no_upstream, IconKind::NoUpstream, "\u{f059}", "?"),
            (icon_synced, IconKind::Synced, "\u{f00c}", "="),
            (
                icon_merged_into_default,
                IconKind::MergedIntoDefault,
                "\u{f058}",
                "M",
            ),
            (
                icon_open_vs_default,
                IconKind::OpenVsDefault,
                "\u{f1bb}",
                "o",
            ),
            (icon_status_failed, IconKind::StatusFailed, "\u{f071}", "!"),
            (icon_viewed, IconKind::Viewed, "\u{f06e}", "*"),
            (icon_comment, IconKind::Comment, "\u{f075}", "\""),
            (
                icon_comment_resolved,
                IconKind::CommentResolved,
                "\u{f0e5}",
                "'",
            ),
        ];
        for (f, kind, nerd, ascii) in fns {
            assert_eq!(f(false), nerd, "{kind:?}");
            assert_eq!(f(true), ascii, "{kind:?}");
            assert_eq!(f(false), kind.glyph(false), "{kind:?}");
            assert_eq!(f(true), kind.glyph(true), "{kind:?}");
        }
        for (f, kind) in [
            (icon_pr_open as IconFn, IconKind::PrOpen),
            (icon_pr_approved, IconKind::PrApproved),
            (icon_pr_merged, IconKind::PrMerged),
        ] {
            assert_eq!(f(false), kind.glyph(false));
            assert_eq!(f(true), kind.glyph(true));
        }
        assert_eq!(IconKind::FileType.glyph(false), DEFAULT_FILE_GLYPH);
        assert_eq!(IconKind::FileType.glyph(true), ASCII_FILE_GLYPH);
        assert_eq!(
            file_icon(true, "a.wat").glyph,
            IconKind::FileType.glyph(true)
        );
        assert_eq!(
            file_icon(false, "a.wat").glyph,
            IconKind::FileType.glyph(false)
        );
    }

    /// Kinds whose glyph is sample text, not one icon cell: two-column
    /// status letters, bracketed chips, and the rails sample.
    fn is_sample_text(kind: IconKind) -> bool {
        matches!(
            kind,
            IconKind::StatusStagedModified
                | IconKind::ChipLocal
                | IconKind::ChipDefault
                | IconKind::ChipRemote
                | IconKind::ChipTag
                | IconKind::ChipDetachedHead
                | IconKind::ChipOverflow
                | IconKind::GraphRails
                | IconKind::WorktreeCount
        )
    }

    #[test]
    fn catalog_glyphs_are_one_column_and_never_emoji() {
        for row in ICON_CATALOG {
            for ascii in [false, true] {
                let g = row.glyph(ascii);
                assert!(!g.is_empty(), "{row:?}");
                assert!(!has_wide_emoji(g), "{row:?}");
                // One column per char: no wide glyph hides in a sample.
                assert_eq!(visible_width(g), g.chars().count(), "{row:?}");
                if !is_sample_text(row.kind) {
                    assert_eq!(visible_width(g), 1, "{row:?} ascii={ascii}");
                }
            }
        }
    }

    /// Status letter rows match the badge letters the tree paints.
    #[test]
    fn status_letter_rows_match_the_badges() {
        for (letter, kind) in [
            (FileStatusLetter::A, IconKind::StatusAdded),
            (FileStatusLetter::S, IconKind::StatusStaged),
            (FileStatusLetter::Ms, IconKind::StatusStagedModified),
            (FileStatusLetter::M, IconKind::StatusModified),
            (FileStatusLetter::D, IconKind::StatusDeleted),
            (FileStatusLetter::R, IconKind::StatusRenamed),
            (FileStatusLetter::U, IconKind::StatusConflict),
            (FileStatusLetter::C, IconKind::StatusCopied),
        ] {
            assert_eq!(letter.icon_kind(), kind);
            assert_eq!(kind.glyph(false), letter.as_str());
            assert_eq!(kind.glyph(true), letter.as_str());
            assert_eq!(kind.spec().group, Some(IconGroup::Tree));
        }
    }

    /// Graph rows read the graph crate's glyph sets. The graph header's
    /// ahead / behind marks are the tree's catalog glyphs in both modes; the
    /// footer's more-lines hint has its own glyph.
    #[test]
    fn graph_rows_match_the_graph_glyph_set() {
        use workspace_status_graph::{GlyphSet, ASCII, UNICODE};
        type Field = fn(&GlyphSet) -> &'static str;
        let pairs: [(IconKind, Field); 10] = [
            (IconKind::GraphCommit, |g| g.commit),
            (IconKind::GraphHeadCommit, |g| g.head_commit),
            (IconKind::GraphStash, |g| g.stash),
            (IconKind::GraphUncommitted, |g| g.uncommitted),
            (IconKind::LinkedWorktree, |g| g.worktree),
            (IconKind::ChipCheckout, |g| g.checkout_mark),
            (IconKind::ChipSynced, |g| g.sync_mark),
            (IconKind::Ahead, |g| g.ahead),
            (IconKind::Behind, |g| g.behind),
            (IconKind::GraphMoreBelow, |g| g.more_below),
        ];
        for (kind, field) in pairs {
            assert_eq!(kind.glyph(false), field(&UNICODE), "{kind:?}");
            assert_eq!(kind.glyph(true), field(&ASCII), "{kind:?}");
        }
        assert_ne!(
            IconKind::GraphMoreBelow.glyph(false),
            IconKind::Behind.glyph(false),
            "more lines below is not behind upstream"
        );
        for (set, ascii) in [(UNICODE, false), (ASCII, true)] {
            let rails = [
                set.vertical,
                set.horizontal,
                set.corner_down_right,
                set.corner_down_left,
                set.cross,
            ]
            .concat();
            assert_eq!(IconKind::GraphRails.glyph(ascii), rails);
        }
    }

    /// Glyphs that paint in the same place differ in both modes. A pair
    /// that shares a glyph (ASCII `#` workspace and Staged, `~` ignored and
    /// Changes, `o` open-vs-default and graph uncommitted) sits in
    /// different contexts, where position tells them apart.
    #[test]
    fn glyphs_are_unique_within_one_paint_context() {
        use IconKind as K;
        let contexts: [(&str, &[IconKind]); 8] = [
            (
                "tree workspace / group row",
                &[K::Workspace, K::Clean, K::Comment, K::CommentResolved],
            ),
            (
                "tree repo / checkout row",
                &[
                    K::Repo,
                    K::LinkedWorktree,
                    K::Branch,
                    K::MergedIntoDefault,
                    K::OpenVsDefault,
                    K::PrOpen,
                    K::PrApproved,
                    K::PrMerged,
                    K::Ahead,
                    K::Behind,
                    K::Diverged,
                    K::NoUpstream,
                    K::Clean,
                    K::StatusFailed,
                    K::Ignored,
                    K::ChangeCount,
                    K::WorktreeCount,
                    K::Comment,
                    K::CommentResolved,
                ],
            ),
            ("tree section row", &[K::Staged, K::Changes]),
            (
                "tree file / dir row",
                &[
                    K::Folder,
                    K::FileType,
                    K::StatusAdded,
                    K::StatusStaged,
                    K::StatusStagedModified,
                    K::StatusModified,
                    K::StatusDeleted,
                    K::StatusRenamed,
                    K::StatusConflict,
                    K::StatusCopied,
                    K::Viewed,
                    K::Comment,
                    K::CommentResolved,
                ],
            ),
            (
                "graph gutter and label",
                &[
                    K::GraphCommit,
                    K::GraphHeadCommit,
                    K::GraphStash,
                    K::GraphUncommitted,
                    K::LinkedWorktree,
                    K::ChipCheckout,
                    K::ChipSynced,
                    K::ChipDetachedHead,
                    K::ChipOverflow,
                    K::PrOpen,
                    K::PrApproved,
                    K::PrMerged,
                    K::Comment,
                    K::CommentResolved,
                ],
            ),
            ("graph sync header", &[K::Ahead, K::Behind]),
            (
                "graph selection footer",
                &[
                    K::ChipCheckout,
                    K::ChipSynced,
                    K::ChipDetachedHead,
                    K::GraphMoreBelow,
                ],
            ),
            (
                "list chrome",
                &[
                    K::FoldExpanded,
                    K::FoldCollapsed,
                    K::CursorBar,
                    K::CursorBarInactive,
                ],
            ),
        ];
        // Branch, default, remote, and tag chips share a bracketed sample;
        // the graph tells them apart by colour, so no glyph check applies.
        let by_colour = [K::ChipLocal, K::ChipDefault, K::ChipRemote, K::ChipTag];
        // Every tree and graph legend row paints somewhere, so each sits in
        // a context.
        for spec in ICON_CATALOG
            .iter()
            .filter(|spec| matches!(spec.group, Some(IconGroup::Tree) | Some(IconGroup::Graph)))
        {
            assert!(
                by_colour.contains(&spec.kind)
                    || contexts.iter().any(|(_, kinds)| kinds.contains(&spec.kind)),
                "{:?} is in no paint context",
                spec.kind
            );
        }
        for (name, kinds) in contexts {
            for ascii in [false, true] {
                let mut seen = std::collections::HashMap::new();
                for &kind in kinds {
                    if let Some(other) = seen.insert(kind.glyph(ascii), kind) {
                        panic!("{name} ascii={ascii}: {kind:?} and {other:?} share a glyph");
                    }
                }
            }
        }
        // The shared pairs named above, and the latent Clean / Synced clash.
        assert_eq!(K::Workspace.glyph(true), K::Staged.glyph(true));
        assert_eq!(K::Ignored.glyph(true), K::Changes.glyph(true));
        assert_eq!(
            K::OpenVsDefault.glyph(true),
            K::GraphUncommitted.glyph(true)
        );
        assert_eq!(K::Clean.glyph(false), K::Synced.glyph(false));
        assert_eq!(K::Synced.spec().group, None, "synced is not painted");
    }

    /// Meanings are one short plain line; the legend lists groups in order.
    #[test]
    fn meanings_are_short_lines_and_groups_are_contiguous() {
        for row in ICON_CATALOG {
            assert!(!row.name.is_empty() && row.name.len() <= 16, "{row:?}");
            assert!(
                !row.meaning.is_empty() && row.meaning.len() <= 56,
                "{row:?}"
            );
            assert!(!row.meaning.contains('\n'), "{row:?}");
            assert!(!row.meaning.ends_with('.'), "no closing period: {row:?}");
        }
        let groups: Vec<IconGroup> = ICON_CATALOG.iter().filter_map(|r| r.group).collect();
        let mut order = groups.clone();
        order.dedup();
        assert_eq!(
            order,
            [IconGroup::Tree, IconGroup::Graph, IconGroup::Chrome],
            "legend groups are contiguous, Tree / Graph / Chrome"
        );
        for kind in [IconKind::HelpMove, IconKind::HelpView, IconKind::FolderOpen] {
            assert_eq!(kind.spec().group, None, "{kind:?}");
        }
    }

    #[test]
    fn pr_badge_glyphs_are_distinct_and_never_words() {
        assert_eq!(icon_pr_open(false), "\u{f407}");
        assert_eq!(icon_pr_approved(false), "\u{f42e}");
        assert_eq!(icon_pr_merged(false), "\u{f419}");
        assert_eq!(
            [
                icon_pr_open(true),
                icon_pr_approved(true),
                icon_pr_merged(true)
            ],
            ["P", "A", "m"]
        );
        for ascii in [true, false] {
            let marks = [
                icon_pr_open(ascii),
                icon_pr_approved(ascii),
                icon_pr_merged(ascii),
            ];
            for mark in marks {
                assert_eq!(visible_width(mark), 1, "{mark:?}");
                // Graph nodes and the tab close glyph.
                for taken in ["●", "◇", "✗"] {
                    assert_ne!(mark, taken);
                }
            }
            assert_ne!(marks[0], marks[1]);
            assert_ne!(marks[1], marks[2]);
            assert_ne!(marks[0], marks[2]);
            assert_ne!(icon_pr_approved(ascii), icon_merged_into_default(ascii));
        }
    }

    #[test]
    fn viewed_is_nf_fa_eye_not_a_substitute() {
        assert_eq!(icon_viewed(false), "\u{f06e}");
        assert_eq!(icon_viewed(true), "*");
        assert_eq!(icon_viewed(false), ICON_VIEWED_NERD);
        assert_eq!(icon_viewed(true), ICON_VIEWED_ASCII);
        let nerd = icon_viewed(false).chars().next().expect("glyph");
        assert_eq!(u32::from(nerd), 0xf06e);
        assert_ne!(nerd, '\u{25c9}'); // ◉
        assert_ne!(nerd, '\u{f07a}'); // other PUA eye/search lookalike
        assert_eq!(visible_width(icon_viewed(false)), 1);
    }

    #[test]
    fn file_type_names_follow_the_devicon_rule() {
        assert_eq!(file_type_name("web/package.json"), "package.json");
        assert_eq!(file_type_name("README.md"), "README.md");
        assert_eq!(file_type_name("docs/readme.md"), "readme.md");
        assert_eq!(file_type_name("src/MAIN.RS"), "rs");
        assert_eq!(file_type_name("src/lib.rs"), "rs");
        assert_eq!(file_type_name("notes.xyz"), "xyz");
        assert_eq!(file_type_name("LICENSE"), "plain");
        assert_eq!(file_type_name(".hidden"), "plain");
    }

    #[test]
    fn file_icons_and_ascii_fallback() {
        assert_eq!(file_icon(true, "a.ts").glyph, "·");
        assert_eq!(visible_width(file_icon(false, "a.ts").glyph), 1);
        assert_eq!(visible_width(file_icon(false, "a.wat").glyph), 1);
        assert_ne!(
            file_icon(false, "package.json").glyph,
            file_icon(false, "tsconfig.json").glyph
        );
        assert_eq!(
            file_icon(false, "README.md").glyph,
            file_icon(false, "readme.md").glyph
        );
    }

    #[test]
    fn sync_and_merge_marks() {
        assert_eq!(
            tui_sync_mark(false, SyncStatus::Behind, "behind by 3"),
            format!("{}3", icon_behind(false))
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::Ahead, "ahead by 2"),
            "^2".to_string()
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::NoUpstream, ""),
            "?".to_string()
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::Diverged, ""),
            "Y".to_string()
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::UpToDate, ""),
            "=".to_string()
        );
        assert_eq!(tui_merge_mark(true, Some(true)), "M");
        assert_eq!(tui_merge_mark(true, Some(false)), "o");
        assert_eq!(tui_merge_mark(true, None), "");
    }
}
